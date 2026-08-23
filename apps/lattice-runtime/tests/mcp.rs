use std::cell::{Cell, RefCell};
use std::io::Cursor;
use std::rc::Rc;

use lattice_runtime::composition::fixed_gateway_submission;
use lattice_runtime::mcp::{
    DeliveryToolArguments, DeliveryToolService, MAX_STDIO_MESSAGE_BYTES,
    MAX_TOOL_INVOCATIONS_PER_SESSION, McpServer, StdioLifecycleEvent, TaskStatusArguments,
    TaskSubmitArguments, ToolExecutionError, serve, serve_legacy_delivery_observer,
    serve_with_lifecycle_observer,
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

    fn task_submit(
        &mut self,
        _arguments: &TaskSubmitArguments,
    ) -> Result<Value, ToolExecutionError> {
        Ok(completed_task_status())
    }

    fn task_status(
        &mut self,
        _arguments: &TaskStatusArguments,
    ) -> Result<Value, ToolExecutionError> {
        Ok(completed_task_status())
    }
}

const CLIENT_REQUEST_ID: &str = "chatgpt-canary-001";
const CONTROLLED_CODEX_CANARY: &str = "CONTROLLED_CODEX_CANARY";
const TASK_REF: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const LEDGER_HEAD_DIGEST: &str = "123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef0";
const RESULT_DIGEST: &str = "23456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef01";

fn completed_task_status() -> Value {
    json!({
        "schema_version": "lattice.task.status.v2",
        "status": "COMPLETED",
        "task_state": "COMPLETED",
        "task_ref": TASK_REF,
        "ledger_head_digest": LEDGER_HEAD_DIGEST,
        "result_digest": RESULT_DIGEST,
        "failure_stage": null,
        "failure_code": null
    })
}

#[test]
fn lifecycle_diagnostics_observe_fixed_mcp_milestones_without_changing_stdout() {
    let service = FakeService {
        run_calls: Rc::new(Cell::new(0)),
        status_calls: Rc::new(Cell::new(0)),
    };
    let input = concat!(
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-11-25\",\"capabilities\":{},\"clientInfo\":{\"name\":\"ignored-secret\",\"version\":\"1\"}}}\n",
        "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\",\"params\":{}}\n"
    );
    let mut output = Vec::new();
    let mut events = Vec::new();

    serve_with_lifecycle_observer(
        service,
        fixed_binding().clone(),
        Cursor::new(input.as_bytes()),
        &mut output,
        |event| events.push(event),
    )
    .expect("bounded MCP stream");

    assert_eq!(
        events,
        vec![
            StdioLifecycleEvent::WaitingForInput,
            StdioLifecycleEvent::InitializeReceived,
            StdioLifecycleEvent::InitializedNotificationReceived,
            StdioLifecycleEvent::ToolsListReceived,
            StdioLifecycleEvent::EndOfStream,
        ]
    );
    let stdout = String::from_utf8(output).expect("MCP stdout is UTF-8 JSONL");
    assert!(!stdout.contains("ignored-secret"));
    let responses = stdout
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("MCP response JSON"))
        .collect::<Vec<_>>();
    assert_eq!(responses.len(), 2);
    assert_eq!(responses[0]["result"]["protocolVersion"], "2025-11-25");
    assert_eq!(
        responses[1]["result"]["tools"].as_array().map(Vec::len),
        Some(6)
    );
}

fn task_public_output_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "schema_version": {
                "type": "string",
                "enum": ["lattice.task.status.v2"]
            },
            "status": {
                "type": "string",
                "enum": [
                    "NOT_SUBMITTED",
                    "RECONCILIATION_REQUIRED",
                    "FAILED",
                    "COMPLETED"
                ]
            },
            "task_state": {
                "type": "string",
                "enum": [
                    "NOT_SUBMITTED",
                    "DRAFT",
                    "AWAITING_EXECUTION_APPROVAL",
                    "PREPARING",
                    "EXECUTING",
                    "VERIFYING",
                    "REVIEWING",
                    "AWAITING_MERGE_APPROVAL",
                    "MERGING",
                    "COMPLETED",
                    "REJECTED",
                    "BLOCKED",
                    "FAILED",
                    "STOPPING",
                    "CANCELLED"
                ]
            },
            "task_ref": {
                "type": "string",
                "minLength": 64,
                "maxLength": 64,
                "pattern": "^[0-9a-f]{64}$"
            },
            "ledger_head_digest": {
                "type": "string",
                "minLength": 64,
                "maxLength": 64,
                "pattern": "^[0-9a-f]{64}$"
            },
            "result_digest": {
                "anyOf": [
                    {
                        "type": "string",
                        "minLength": 64,
                        "maxLength": 64,
                        "pattern": "^[0-9a-f]{64}$"
                    },
                    {"type": "null"}
                ]
            },
            "failure_stage": {
                "anyOf": [
                    {"type": "string", "minLength": 1, "maxLength": 128, "pattern": "^[A-Z0-9_]+$"},
                    {"type": "null"}
                ]
            },
            "failure_code": {
                "anyOf": [
                    {"type": "string", "minLength": 1, "maxLength": 128, "pattern": "^[A-Z0-9_]+$"},
                    {"type": "null"}
                ]
            }
        },
        "required": [
            "schema_version",
            "status",
            "task_state",
            "task_ref",
            "ledger_head_digest",
            "result_digest",
            "failure_stage",
            "failure_code"
        ],
        "additionalProperties": false
    })
}

#[derive(Clone)]
struct CapturingTaskService {
    submits: Rc<RefCell<Vec<TaskSubmitArguments>>>,
    statuses: Rc<RefCell<Vec<TaskStatusArguments>>>,
}

impl DeliveryToolService for CapturingTaskService {
    fn run(&mut self, _arguments: &DeliveryToolArguments) -> Result<Value, ToolExecutionError> {
        panic!("delivery run must not be dispatched by task-tool tests")
    }

    fn status(&mut self, _arguments: &DeliveryToolArguments) -> Result<Value, ToolExecutionError> {
        panic!("delivery status must not be dispatched by task-tool tests")
    }

    fn task_submit(
        &mut self,
        arguments: &TaskSubmitArguments,
    ) -> Result<Value, ToolExecutionError> {
        self.submits.borrow_mut().push(arguments.clone());
        Ok(completed_task_status())
    }

    fn task_status(
        &mut self,
        arguments: &TaskStatusArguments,
    ) -> Result<Value, ToolExecutionError> {
        self.statuses.borrow_mut().push(arguments.clone());
        Ok(completed_task_status())
    }
}

type TaskServerFixture = (
    McpServer<CapturingTaskService>,
    Rc<RefCell<Vec<TaskSubmitArguments>>>,
    Rc<RefCell<Vec<TaskStatusArguments>>>,
);

fn task_server() -> TaskServerFixture {
    let submits = Rc::new(RefCell::new(Vec::new()));
    let statuses = Rc::new(RefCell::new(Vec::new()));
    let service = CapturingTaskService {
        submits: submits.clone(),
        statuses: statuses.clone(),
    };
    (
        McpServer::new(service, fixed_binding().clone()),
        submits,
        statuses,
    )
}

fn legacy_delivery_observer_task_server() -> TaskServerFixture {
    let submits = Rc::new(RefCell::new(Vec::new()));
    let statuses = Rc::new(RefCell::new(Vec::new()));
    let service = CapturingTaskService {
        submits: submits.clone(),
        statuses: statuses.clone(),
    };
    (
        McpServer::new_legacy_delivery_observer(service, fixed_binding().clone()),
        submits,
        statuses,
    )
}

struct FixedTaskOutputService {
    output: Value,
}

impl DeliveryToolService for FixedTaskOutputService {
    fn run(&mut self, _arguments: &DeliveryToolArguments) -> Result<Value, ToolExecutionError> {
        panic!("delivery run must not be dispatched by task-output tests")
    }

    fn status(&mut self, _arguments: &DeliveryToolArguments) -> Result<Value, ToolExecutionError> {
        panic!("delivery status must not be dispatched by task-output tests")
    }

    fn task_submit(
        &mut self,
        _arguments: &TaskSubmitArguments,
    ) -> Result<Value, ToolExecutionError> {
        Ok(self.output.clone())
    }

    fn task_status(
        &mut self,
        _arguments: &TaskStatusArguments,
    ) -> Result<Value, ToolExecutionError> {
        Ok(self.output.clone())
    }
}

fn call_task_tool_with_output(tool_name: &str, output: Value) -> Value {
    let mut server = McpServer::new(FixedTaskOutputService { output }, fixed_binding().clone());
    initialize(&mut server);
    let arguments = match tool_name {
        "lattice_task_submit" => json!({
            "client_request_id": CLIENT_REQUEST_ID,
            "intent": CONTROLLED_CODEX_CANARY
        }),
        "lattice_task_status" => json!({
            "client_request_id": CLIENT_REQUEST_ID,
            "task_ref": TASK_REF
        }),
        _ => panic!("unexpected task tool"),
    };
    server
        .handle(json!({
            "jsonrpc": "2.0",
            "id": "task-output",
            "method": "tools/call",
            "params": {"name": tool_name, "arguments": arguments}
        }))
        .expect("task output response")
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

fn initialize<S: DeliveryToolService>(server: &mut McpServer<S>) {
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
        [
            "lattice_delivery_run",
            "lattice_delivery_status",
            "lattice_task_submit",
            "lattice_task_status",
            "lattice_runtime_status",
            "lattice_delivery_reconcile"
        ]
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
    assert_eq!(
        list["result"]["tools"][2]["annotations"],
        json!({
            "readOnlyHint": false,
            "destructiveHint": true,
            "idempotentHint": true,
            "openWorldHint": false
        })
    );
    assert_eq!(
        list["result"]["tools"][3]["annotations"],
        json!({
            "readOnlyHint": true,
            "destructiveHint": false,
            "idempotentHint": true,
            "openWorldHint": false
        })
    );
    for tool in &list["result"]["tools"].as_array().expect("tools")[2..4] {
        assert_eq!(tool["outputSchema"], task_public_output_schema());
    }

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
fn modern_stateless_calls_share_the_bounded_process_rate_counter() {
    let (mut server, run_calls, status_calls) = server();
    for id in 0..MAX_TOOL_INVOCATIONS_PER_SESSION {
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
    let rejected = server
        .handle(json!({
            "jsonrpc": "2.0",
            "id": "over-limit",
            "method": "tools/call",
            "params": {
                "name": "lattice_delivery_status",
                "arguments": {},
                "_meta": modern_request_meta()
            }
        }))
        .expect("modern rate-limit response");
    assert_eq!(rejected["error"]["code"], -32029);
    assert_eq!(run_calls.get(), 0);
    assert_eq!(
        status_calls.get() as usize,
        MAX_TOOL_INVOCATIONS_PER_SESSION
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
        Some(6)
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
fn tool_list_is_exactly_six_bounded_tools_with_closed_schemas() {
    let (mut server, _, _) = server();
    initialize(&mut server);

    let response = server
        .handle(json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}))
        .expect("tool list");
    let tools = response["result"]["tools"].as_array().expect("tools");

    assert_eq!(tools.len(), 6);
    assert_eq!(
        tools
            .iter()
            .map(|tool| tool["name"].as_str().expect("name"))
            .collect::<Vec<_>>(),
        [
            "lattice_delivery_run",
            "lattice_delivery_status",
            "lattice_task_submit",
            "lattice_task_status",
            "lattice_runtime_status",
            "lattice_delivery_reconcile"
        ]
    );
    for tool in &tools[..2] {
        assert_eq!(
            tool["inputSchema"],
            json!({"type":"object","additionalProperties":false})
        );
        assert!(tool.get("outputSchema").is_none());
        assert!(tool.get("annotations").is_none());
    }
    assert_eq!(
        tools[4]["inputSchema"],
        json!({"type":"object","additionalProperties":false})
    );
    assert_eq!(
        tools[5]["inputSchema"],
        json!({"type":"object","additionalProperties":false})
    );
    let task_output_schema = task_public_output_schema();
    assert_eq!(
        tools[2]["inputSchema"],
        json!({
            "type": "object",
            "properties": {
                "client_request_id": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 64,
                    "pattern": "^[A-Za-z0-9][A-Za-z0-9._:-]{0,63}$"
                },
                "intent": {
                    "type": "string",
                    "enum": ["CONTROLLED_CODEX_CANARY"]
                }
            },
            "required": ["client_request_id", "intent"],
            "additionalProperties": false
        })
    );
    assert_eq!(
        tools[3]["inputSchema"],
        json!({
            "type": "object",
            "properties": {
                "client_request_id": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 64,
                    "pattern": "^[A-Za-z0-9][A-Za-z0-9._:-]*$"
                },
                "task_ref": {
                    "type": "string",
                    "minLength": 64,
                    "maxLength": 64,
                    "pattern": "^[0-9a-f]{64}$"
                }
            },
            "required": ["client_request_id", "task_ref"],
            "additionalProperties": false
        })
    );
    assert_eq!(tools[2]["outputSchema"], task_output_schema);
    assert_eq!(tools[3]["outputSchema"], task_output_schema);
    assert!(tools[2].get("annotations").is_none());
    assert!(tools[3].get("annotations").is_none());
}

#[test]
fn legacy_observer_neither_mutates_nor_advertises_or_dispatches_task_tools() {
    let (mut legacy_server, submits, statuses) = legacy_delivery_observer_task_server();
    initialize(&mut legacy_server);

    let legacy_list = legacy_server
        .handle(json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}))
        .expect("legacy delivery-only tool list");
    assert_eq!(
        legacy_list["result"]["tools"]
            .as_array()
            .expect("legacy tools")
            .iter()
            .map(|tool| tool["name"].as_str().expect("tool name"))
            .collect::<Vec<_>>(),
        ["lattice_delivery_run", "lattice_delivery_status"]
    );

    let disabled_run = legacy_server
        .handle(json!({
            "jsonrpc": "2.0",
            "id": "legacy-run",
            "method": "tools/call",
            "params": {"name": "lattice_delivery_run", "arguments": {}}
        }))
        .expect("legacy disabled run response");
    assert_eq!(disabled_run["result"]["isError"], true);
    assert_eq!(
        disabled_run["result"]["structuredContent"],
        json!({
            "status": "ERROR",
            "code": "LATTICE_DELIVERY_RUN_REQUIRES_CANONICAL_LATTICED"
        })
    );

    for (id, name, arguments) in [
        (
            "legacy-submit",
            "lattice_task_submit",
            json!({
                "client_request_id": CLIENT_REQUEST_ID,
                "intent": CONTROLLED_CODEX_CANARY
            }),
        ),
        (
            "legacy-status",
            "lattice_task_status",
            json!({"task_ref": TASK_REF}),
        ),
    ] {
        let response = legacy_server
            .handle(json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "tools/call",
                "params": {"name": name, "arguments": arguments}
            }))
            .expect("legacy disabled task response");
        assert_eq!(response["error"]["code"], -32602, "{name}");
        assert_eq!(response["error"]["message"], "Unknown tool", "{name}");
    }
    assert!(submits.borrow().is_empty());
    assert!(statuses.borrow().is_empty());
}

#[test]
fn stateless_legacy_observer_neither_advertises_nor_dispatches_task_tools() {
    let (mut stateless_server, stateless_submits, stateless_statuses) =
        legacy_delivery_observer_task_server();
    let stateless_list = stateless_server
        .handle(json!({
            "jsonrpc": "2.0",
            "id": "stateless-list",
            "method": "tools/list",
            "params": {"_meta": modern_request_meta()}
        }))
        .expect("stateless delivery-only tool list");
    let stateless_tools = stateless_list["result"]["tools"]
        .as_array()
        .expect("stateless tools");
    assert_eq!(
        stateless_tools
            .iter()
            .map(|tool| tool["name"].as_str().expect("tool name"))
            .collect::<Vec<_>>(),
        ["lattice_delivery_run", "lattice_delivery_status"]
    );
    assert!(
        stateless_tools
            .iter()
            .all(|tool| tool.get("annotations").is_some())
    );

    for (id, name, arguments) in [
        (
            "stateless-submit",
            "lattice_task_submit",
            json!({
                "client_request_id": CLIENT_REQUEST_ID,
                "intent": CONTROLLED_CODEX_CANARY
            }),
        ),
        (
            "stateless-status",
            "lattice_task_status",
            json!({"task_ref": TASK_REF}),
        ),
    ] {
        let response = stateless_server
            .handle(json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "tools/call",
                "params": {
                    "name": name,
                    "arguments": arguments,
                    "_meta": modern_request_meta()
                }
            }))
            .expect("stateless disabled task response");
        assert_eq!(response["error"]["code"], -32602, "{name}");
        assert_eq!(response["error"]["message"], "Unknown tool", "{name}");
    }
    assert!(stateless_submits.borrow().is_empty());
    assert!(stateless_statuses.borrow().is_empty());
}

#[test]
fn legacy_observer_stdio_transport_uses_the_restricted_catalog() {
    let run_calls = Rc::new(Cell::new(0));
    let status_calls = Rc::new(Cell::new(0));
    let service = FakeService {
        run_calls: run_calls.clone(),
        status_calls: status_calls.clone(),
    };
    let input = concat!(
        "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-11-25\",\"capabilities\":{},\"clientInfo\":{}}}\n",
        "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\",\"params\":{}}\n",
        "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"lattice_delivery_status\",\"arguments\":{}}}\n"
    );
    let mut output = Vec::new();

    serve_legacy_delivery_observer(
        service,
        fixed_binding().clone(),
        Cursor::new(input.as_bytes()),
        &mut output,
    )
    .expect("serve legacy observer");

    let responses = String::from_utf8(output)
        .expect("utf8")
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).expect("json response"))
        .collect::<Vec<_>>();
    assert_eq!(
        responses[1]["result"]["tools"]
            .as_array()
            .expect("tools")
            .iter()
            .map(|tool| tool["name"].as_str().expect("tool name"))
            .collect::<Vec<_>>(),
        ["lattice_delivery_run", "lattice_delivery_status"]
    );
    assert_eq!(responses[2]["result"]["isError"], false);
    assert_eq!(
        responses[2]["result"]["structuredContent"]["kind"],
        "status"
    );
    assert_eq!(run_calls.get(), 0);
    assert_eq!(status_calls.get(), 1);
}

#[test]
fn bounded_task_tools_dispatch_only_typed_arguments() {
    let (mut server, submits, statuses) = task_server();
    initialize(&mut server);

    let submit = server
        .handle(json!({
            "jsonrpc": "2.0",
            "id": "submit",
            "method": "tools/call",
            "params": {
                "name": "lattice_task_submit",
                "arguments": {
                    "client_request_id": CLIENT_REQUEST_ID,
                    "intent": CONTROLLED_CODEX_CANARY
                }
            }
        }))
        .expect("task submit response");
    assert_eq!(submit["result"]["isError"], false);
    assert_eq!(submit["result"]["structuredContent"]["task_ref"], TASK_REF);

    let status = server
        .handle(json!({
            "jsonrpc": "2.0",
            "id": "status",
            "method": "tools/call",
            "params": {
                "name": "lattice_task_status",
                "arguments": {"client_request_id": CLIENT_REQUEST_ID, "task_ref": TASK_REF}
            }
        }))
        .expect("task status response");
    assert_eq!(status["result"]["isError"], false);

    let submits = submits.borrow();
    assert_eq!(submits.len(), 1);
    assert_eq!(submits[0].client_request_id(), CLIENT_REQUEST_ID);
    assert_eq!(submits[0].intent(), CONTROLLED_CODEX_CANARY);
    let statuses = statuses.borrow();
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0].task_ref(), TASK_REF);
}

#[test]
fn task_tools_emit_only_closed_public_status_shapes() {
    let accepted = [
        completed_task_status(),
        json!({
            "schema_version": "lattice.task.status.v2",
            "status": "FAILED",
            "task_state": "FAILED",
            "task_ref": TASK_REF,
            "ledger_head_digest": LEDGER_HEAD_DIGEST,
            "result_digest": null,
            "failure_stage": "CODEX",
            "failure_code": "LATTICE_DELIVERY_FAILED"
        }),
        json!({
            "schema_version": "lattice.task.status.v2",
            "status": "RECONCILIATION_REQUIRED",
            "task_state": "EXECUTING",
            "task_ref": TASK_REF,
            "ledger_head_digest": LEDGER_HEAD_DIGEST,
            "result_digest": null,
            "failure_stage": "OUTCOME",
            "failure_code": "LATTICE_DELIVERY_RECONCILIATION_REQUIRED"
        }),
    ];

    for tool_name in ["lattice_task_submit", "lattice_task_status"] {
        for output in &accepted {
            let response = call_task_tool_with_output(tool_name, output.clone());
            assert_eq!(response["result"]["isError"], false, "{tool_name}");
            assert_eq!(
                response["result"]["structuredContent"], *output,
                "{tool_name}"
            );
        }
    }
}

#[test]
fn task_tools_fail_closed_before_serializing_invalid_public_status() {
    let mut invalid = vec![Value::Null, json!([]), json!({})];

    for field in [
        "schema_version",
        "status",
        "task_state",
        "task_ref",
        "ledger_head_digest",
        "result_digest",
        "failure_stage",
        "failure_code",
    ] {
        let mut value = completed_task_status();
        value.as_object_mut().expect("status object").remove(field);
        invalid.push(value);
    }

    for (field, replacement) in [
        ("schema_version", json!(1)),
        ("status", json!(false)),
        ("task_state", json!([])),
        ("task_ref", json!(7)),
        ("ledger_head_digest", Value::Null),
        ("result_digest", json!({})),
    ] {
        let mut value = completed_task_status();
        value[field] = replacement;
        invalid.push(value);
    }

    for (field, replacement) in [
        ("schema_version", "lattice.task.status.v1"),
        ("status", "ACCEPTED"),
        ("task_state", "UNKNOWN"),
        ("task_ref", "a"),
        (
            "ledger_head_digest",
            "A123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        ),
        ("result_digest", "abc"),
    ] {
        let mut value = completed_task_status();
        value[field] = json!(replacement);
        invalid.push(value);
    }

    for field in [
        "extra",
        "actor_id",
        "lease",
        "fence",
        "path",
        "command",
        "codex_thread",
    ] {
        let mut value = completed_task_status();
        value
            .as_object_mut()
            .expect("status object")
            .insert(field.to_owned(), json!("forbidden"));
        invalid.push(value);
    }

    for tool_name in ["lattice_task_submit", "lattice_task_status"] {
        for output in &invalid {
            let response = call_task_tool_with_output(tool_name, output.clone());
            assert_eq!(response["result"]["isError"], true, "{tool_name}: {output}");
            assert_eq!(
                response["result"]["structuredContent"],
                json!({
                    "status": "ERROR",
                    "code": "LATTICE_TASK_PUBLIC_STATUS_REJECTED"
                }),
                "{tool_name}: {output}"
            );
            let text = response["result"]["content"][0]["text"]
                .as_str()
                .expect("bounded error text");
            assert!(!text.contains("forbidden"), "{tool_name}: {output}");
        }
    }
}

#[test]
fn stateless_client_info_cannot_change_typed_task_arguments() {
    let (mut server, submits, statuses) = task_server();

    for (id, client_info) in [
        (
            "first",
            json!({
                "name": "untrusted-first",
                "version": "1",
                "actor_id": "attacker",
                "session_id": "forged-session"
            }),
        ),
        (
            "second",
            json!({
                "name": "untrusted-second",
                "version": "99",
                "actor_id": "different-attacker",
                "session_id": "different-forgery"
            }),
        ),
    ] {
        let mut metadata = modern_request_meta();
        metadata["io.modelcontextprotocol/clientInfo"] = client_info;
        let response = server
            .handle(json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "tools/call",
                "params": {
                    "name": "lattice_task_submit",
                    "arguments": {
                        "client_request_id": CLIENT_REQUEST_ID,
                        "intent": CONTROLLED_CODEX_CANARY
                    },
                    "_meta": metadata
                }
            }))
            .expect("stateless task submit response");
        assert_eq!(response["result"]["resultType"], "complete");
        assert_eq!(response["result"]["isError"], false);
    }

    let submits = submits.borrow();
    assert_eq!(submits.len(), 2);
    assert_eq!(submits[0], submits[1]);
    assert_eq!(submits[0].client_request_id(), CLIENT_REQUEST_ID);
    assert_eq!(submits[0].intent(), CONTROLLED_CODEX_CANARY);

    let status = server
        .handle(json!({
            "jsonrpc": "2.0",
            "id": "stateless-status",
            "method": "tools/call",
            "params": {
                "name": "lattice_task_status",
                "arguments": {"client_request_id": CLIENT_REQUEST_ID, "task_ref": TASK_REF},
                "_meta": modern_request_meta()
            }
        }))
        .expect("stateless task status response");
    assert_eq!(status["result"]["resultType"], "complete");
    assert_eq!(status["result"]["isError"], false);
    let statuses = statuses.borrow();
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0].task_ref(), TASK_REF);
}

#[test]
fn task_submit_rejects_invalid_or_dangerous_arguments_before_dispatch() {
    let invalid_arguments = [
        json!({}),
        json!({"client_request_id": CLIENT_REQUEST_ID}),
        json!({"intent": CONTROLLED_CODEX_CANARY}),
        json!({"client_request_id": "", "intent": CONTROLLED_CODEX_CANARY}),
        json!({"client_request_id": "a".repeat(65), "intent": CONTROLLED_CODEX_CANARY}),
        json!({"client_request_id": "not ascii", "intent": CONTROLLED_CODEX_CANARY}),
        json!({"client_request_id": "非ASCII", "intent": CONTROLLED_CODEX_CANARY}),
        json!({"client_request_id": CLIENT_REQUEST_ID, "intent": "ARBITRARY_TASK"}),
        json!({"client_request_id": 7, "intent": CONTROLLED_CODEX_CANARY}),
        json!({"client_request_id": CLIENT_REQUEST_ID, "intent": false}),
        Value::Null,
        json!([]),
    ];

    for arguments in invalid_arguments {
        let (mut server, submits, statuses) = task_server();
        initialize(&mut server);
        let response = server
            .handle(json!({
                "jsonrpc": "2.0",
                "id": "invalid-submit",
                "method": "tools/call",
                "params": {"name": "lattice_task_submit", "arguments": arguments}
            }))
            .expect("invalid submit response");
        assert_eq!(response["error"]["code"], -32602);
        assert!(submits.borrow().is_empty());
        assert!(statuses.borrow().is_empty());
    }

    for property in [
        "project_id",
        "actor_id",
        "session_id",
        "command_id",
        "approval",
        "shell",
        "sql",
        "path",
        "filesystem_write",
        "git",
        "git_command",
        "credential",
        "secret",
        "provider",
        "lease",
        "writer_lease",
        "thread_id",
        "codex_thread",
        "task",
        "extra",
    ] {
        let (mut server, submits, statuses) = task_server();
        initialize(&mut server);
        let mut arguments = serde_json::Map::from_iter([
            (
                "client_request_id".to_owned(),
                Value::String(CLIENT_REQUEST_ID.to_owned()),
            ),
            (
                "intent".to_owned(),
                Value::String(CONTROLLED_CODEX_CANARY.to_owned()),
            ),
        ]);
        arguments.insert(property.to_owned(), json!("forbidden"));
        let response = server
            .handle(json!({
                "jsonrpc": "2.0",
                "id": property,
                "method": "tools/call",
                "params": {
                    "name": "lattice_task_submit",
                    "arguments": Value::Object(arguments)
                }
            }))
            .expect("dangerous submit response");
        assert_eq!(response["error"]["code"], -32602, "{property}");
        assert!(submits.borrow().is_empty(), "{property}");
        assert!(statuses.borrow().is_empty(), "{property}");
    }
}

#[test]
fn task_status_requires_one_lowercase_sha256_reference_before_dispatch() {
    for arguments in [
        json!({}),
        json!({"task_ref": ""}),
        json!({"task_ref": "a".repeat(63)}),
        json!({"task_ref": "a".repeat(65)}),
        json!({"task_ref": TASK_REF.to_uppercase()}),
        json!({"task_ref": format!("{}g", "a".repeat(63))}),
        json!({"task_ref": 7}),
        json!({"task_ref": TASK_REF, "extra": "forbidden"}),
        json!({"task_ref": TASK_REF, "shell": "forbidden"}),
        Value::Null,
        json!([]),
    ] {
        let (mut server, submits, statuses) = task_server();
        initialize(&mut server);
        let response = server
            .handle(json!({
                "jsonrpc": "2.0",
                "id": "invalid-status",
                "method": "tools/call",
                "params": {"name": "lattice_task_status", "arguments": arguments}
            }))
            .expect("invalid status response");
        assert_eq!(response["error"]["code"], -32602);
        assert!(submits.borrow().is_empty());
        assert!(statuses.borrow().is_empty());
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
    assert_eq!(list["result"]["tools"].as_array().map(Vec::len), Some(6));

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

        fn task_submit(
            &mut self,
            _arguments: &TaskSubmitArguments,
        ) -> Result<Value, ToolExecutionError> {
            Err(ToolExecutionError::new("LATTICE_TASK_SUBMIT_REJECTED"))
        }

        fn task_status(
            &mut self,
            _arguments: &TaskStatusArguments,
        ) -> Result<Value, ToolExecutionError> {
            Err(ToolExecutionError::new("LATTICE_TASK_STATUS_REJECTED"))
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
