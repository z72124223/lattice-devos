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
    fn run(&mut self, _arguments: &DeliveryToolArguments) -> Result<Value, ToolExecutionError> {
        self.run_calls.set(self.run_calls.get() + 1);
        Ok(json!({"status": "COMPLETED", "kind": "run"}))
    }

    fn status(&mut self, _arguments: &DeliveryToolArguments) -> Result<Value, ToolExecutionError> {
        self.status_calls.set(self.status_calls.get() + 1);
        Ok(json!({"status": "COMPLETED", "kind": "status"}))
    }
}

fn tool_arguments() -> Value {
    let submission = fixed_gateway_submission().expect("fixed full-chain submission");
    let binding = submission.binding();
    json!({
        "project_id": binding.project_id().as_str(),
        "project_snapshot_id": binding.project_snapshot_id().as_str(),
        "task_id": binding.task_id().as_str(),
        "revision": binding.task_revision(),
        "task_spec_digest": binding.task_spec_digest().as_str(),
    })
}

fn server() -> (McpServer<FakeService>, Rc<Cell<u32>>, Rc<Cell<u32>>) {
    let run_calls = Rc::new(Cell::new(0));
    let status_calls = Rc::new(Cell::new(0));
    let service = FakeService {
        run_calls: run_calls.clone(),
        status_calls: status_calls.clone(),
    };
    (McpServer::new(service), run_calls, status_calls)
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

#[test]
fn tool_list_is_exactly_two_closed_typed_binding_tools() {
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
    let expected_arguments = tool_arguments();
    for tool in tools {
        let schema = &tool["inputSchema"];
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["additionalProperties"], false);
        assert_eq!(
            schema["required"],
            json!([
                "project_id",
                "project_snapshot_id",
                "task_id",
                "revision",
                "task_spec_digest"
            ])
        );
        for field in [
            "project_id",
            "project_snapshot_id",
            "task_id",
            "revision",
            "task_spec_digest",
        ] {
            assert_eq!(
                schema["properties"][field]["const"],
                expected_arguments[field]
            );
        }
    }
}

#[test]
fn exact_typed_arguments_dispatch_each_fixed_tool() {
    let (mut server, run_calls, status_calls) = server();
    initialize(&mut server);

    for (id, name) in [(2, "lattice_delivery_run"), (3, "lattice_delivery_status")] {
        let response = server
            .handle(json!({
                "jsonrpc":"2.0",
                "id":id,
                "method":"tools/call",
                "params":{"name":name,"arguments":tool_arguments()}
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
                "arguments":tool_arguments(),
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
                "arguments":tool_arguments(),
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
                "params":{"name":"lattice_delivery_status","arguments":tool_arguments()}
            }))
            .expect("tool response within limit");
        assert_eq!(response["result"]["isError"], false);
    }

    let rejected = server
        .handle(json!({
            "jsonrpc":"2.0",
            "id":999,
            "method":"tools/call",
            "params":{"name":"lattice_delivery_run","arguments":tool_arguments()}
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
        "shell",
        "sql",
        "path",
        "credential",
        "provider",
        "task",
        "extra",
    ];

    for property in dangerous {
        let (mut server, run_calls, status_calls) = server();
        initialize(&mut server);
        let mut arguments = tool_arguments().as_object().expect("arguments").clone();
        arguments.insert(property.to_owned(), json!("forbidden"));
        let response = server
            .handle(json!({
                "jsonrpc":"2.0",
                "id":2,
                "method":"tools/call",
                "params":{
                    "name":"lattice_delivery_run",
                    "arguments":Value::Object(arguments)
                }
            }))
            .expect("error response");

        assert_eq!(response["error"]["code"], -32602);
        assert_eq!(run_calls.get(), 0, "{property}");
        assert_eq!(status_calls.get(), 0, "{property}");
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
    let mut server = McpServer::new(FailingService);
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
            "params":{"name":"lattice_delivery_run","arguments":tool_arguments()}
        }))
        .expect("tool result");

    assert_eq!(response["result"]["isError"], true);
    assert_eq!(
        response["result"]["structuredContent"],
        json!({"status":"ERROR","code":"LATTICE_DELIVERY_REJECTED"})
    );
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

    serve(service, Cursor::new(input.as_bytes()), &mut output).expect("serve");

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

    serve(service, Cursor::new(input), &mut output).expect("serve");

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

    serve(service, Cursor::new(input), &mut output).expect("serve");

    let response = serde_json::from_slice::<Value>(&output).expect("json response");
    assert_eq!(response["id"], Value::Null);
    assert_eq!(response["error"]["code"], -32600);
    assert_eq!(response["error"]["message"], "Unterminated message");
    assert_eq!(run_calls.get(), 0);
    assert_eq!(status_calls.get(), 0);
}
