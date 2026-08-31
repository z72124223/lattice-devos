use std::cell::{Cell, RefCell};
use std::io::Cursor;
use std::rc::Rc;

use lattice_foreman_state::DependencyBinding;
use lattice_runtime::composition::fixed_gateway_submission;
use lattice_runtime::mcp::{
    DeliveryToolArguments, DeliveryToolService, ForemanCheckpointArguments,
    MAX_STDIO_MESSAGE_BYTES, MAX_TOOL_INVOCATIONS_PER_SESSION, McpServer, StdioLifecycleEvent,
    TaskStatusArguments, TaskSubmitArguments, ToolExecutionError, serve,
    serve_legacy_delivery_observer, serve_with_lifecycle_observer,
};
use serde_json::{Value, json};

#[derive(Clone)]
struct FakeService {
    run_calls: Rc<Cell<u32>>,
    status_calls: Rc<Cell<u32>>,
}

#[derive(Clone)]
struct CheckpointService {
    calls: Rc<Cell<u32>>,
}

impl DeliveryToolService for CheckpointService {
    fn run(&mut self, _arguments: &DeliveryToolArguments) -> Result<Value, ToolExecutionError> {
        Ok(json!({}))
    }

    fn status(&mut self, _arguments: &DeliveryToolArguments) -> Result<Value, ToolExecutionError> {
        Ok(json!({}))
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

    fn foreman_checkpoint(
        &mut self,
        arguments: &ForemanCheckpointArguments,
    ) -> Result<Value, ToolExecutionError> {
        self.calls.set(self.calls.get() + 1);
        let intent = arguments.intent();
        if let Some(blocker) = intent.blocker_ref()
            && let Some(binding) =
                DependencyBinding::from_blocker_ref(blocker).expect("dependency blocker replay")
        {
            assert_eq!(intent.evidence_ref(), binding.evidence_ref());
        }
        Ok(json!({
            "schema": "lattice.foreman-checkpoint-result/1.0",
            "checkpoint_id": intent.checkpoint_id(),
            "generation": intent.generation(),
            "status": "RECORDED",
            "exact_retry": false,
            "ledger_digest": "1".repeat(64),
            "checkpoint_digest": "2".repeat(64)
        }))
    }
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

fn submitted_general_task_status() -> Value {
    json!({
        "schema_version": "lattice.task.status.v5",
        "status": "SUBMITTED",
        "task_state": "DRAFT",
        "task_ref": TASK_REF,
        "ledger_head_digest": LEDGER_HEAD_DIGEST,
        "result_digest": null,
        "failure_stage": null,
        "failure_code": null,
        "objective_summary": "Objective retained; digest only.",
        "objective_digest": RESULT_DIGEST,
        "project_id": "legacy-project-id",
        "project_name": "AI 劇本",
        "project_snapshot_id": "legacy-project-id:snapshot:1"
    })
}

fn managed_general_task_status() -> Value {
    json!({
        "schema_version": "lattice.task.status.v4",
        "status": "RUNNING",
        "task_state": "EXECUTING",
        "task_ref": TASK_REF,
        "ledger_head_digest": LEDGER_HEAD_DIGEST,
        "result_digest": null,
        "failure_stage": null,
        "failure_code": null,
        "objective_summary": "Objective retained; digest only.",
        "objective_digest": RESULT_DIGEST,
        "project_id": "legacy-project-id",
        "project_name": "AI 劇本",
        "project_snapshot_id": "legacy-project-id:snapshot:1",
        "worker_running": true,
        "attempt": 1,
        "retry_count": 0,
        "model": "gpt-5.6-terra",
        "reasoning": "medium",
        "thread_id": "019c-thread-1",
        "turn_id": "019c-turn-1",
        "last_progress_at": "2026-08-26T12:34:56Z",
        "blocker": null,
        "verification_status": "RUNNING",
        "verification_digest": null,
        "evidence_digest": RESULT_DIGEST,
        "resource_observation": {
            "scope": "TASK_CUMULATIVE",
            "attempts_observed": 1,
            "model_calls": 1,
            "remaining_model_calls": 5,
            "remaining_total_tokens": 99840,
            "input_tokens": 120,
            "cached_input_tokens": 20,
            "output_tokens": 40,
            "reasoning_output_tokens": null,
            "total_tokens": 160,
            "external_cost_status": "UNAVAILABLE"
        },
        "next_action": "Wait for independent verification.",
        "foreman_generation": 2,
        "foreman_checkpoint_digest": LEDGER_HEAD_DIGEST
    })
}

fn valid_foreman_checkpoint_arguments() -> Value {
    json!({
        "checkpoint_id": "checkpoint-1",
        "generation": 1,
        "occurred_at": "2026-08-25T00:00:01Z",
        "state": "ACTIVE",
        "blocker_ref": null,
        "heartbeat_ref": "heartbeat:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "evidence_ref": "evidence:sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
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
        Some(7)
    );
}

fn task_public_output_schema() -> Value {
    json!({
        "oneOf": [
            task_public_output_variant(false),
            redacted_task_public_output_variant(),
            managed_task_public_output_variant()
        ]
    })
}

fn redacted_task_public_output_variant() -> Value {
    let mut schema = task_public_output_variant(true);
    let properties = schema["properties"]
        .as_object_mut()
        .expect("redacted status properties");
    properties.remove("objective");
    properties.insert(
        "objective_summary".to_owned(),
        json!({"type": "string", "enum": ["Objective retained; digest only."]}),
    );
    properties.insert(
        "objective_digest".to_owned(),
        json!({"type": "string", "minLength": 64, "maxLength": 64, "pattern": "^[0-9a-f]{64}$"}),
    );
    properties["schema_version"] = json!({"type": "string", "enum": ["lattice.task.status.v5"]});
    let required = schema["required"]
        .as_array_mut()
        .expect("redacted required fields");
    required.retain(|field| field.as_str() != Some("objective"));
    required.extend(
        ["objective_summary", "objective_digest"]
            .into_iter()
            .map(|field| json!(field)),
    );
    schema
}

fn task_public_output_variant(general: bool) -> Value {
    let sha = || {
        json!({
            "type": "string",
            "minLength": 64,
            "maxLength": 64,
            "pattern": "^[0-9a-f]{64}$"
        })
    };
    let mut properties = json!({
        "schema_version": {
            "type": "string",
            "enum": [if general { "lattice.task.status.v3" } else { "lattice.task.status.v2" }]
        },
        "status": {
            "type": "string",
            "enum": if general {
                json!(["NOT_SUBMITTED", "SUBMITTED", "RECONCILIATION_REQUIRED", "FAILED", "COMPLETED"])
            } else {
                json!(["NOT_SUBMITTED", "RECONCILIATION_REQUIRED", "FAILED", "COMPLETED"])
            }
        },
        "task_state": {
            "type": "string",
            "enum": [
                "NOT_SUBMITTED", "DRAFT", "AWAITING_EXECUTION_APPROVAL", "PREPARING",
                "EXECUTING", "VERIFYING", "REVIEWING", "AWAITING_MERGE_APPROVAL",
                "MERGING", "COMPLETED", "REJECTED", "BLOCKED", "FAILED", "STOPPING",
                "CANCELLED"
            ]
        },
        "task_ref": sha(),
        "ledger_head_digest": sha(),
        "result_digest": {"anyOf": [sha(), {"type": "null"}]},
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
    });
    if general {
        properties["status"] = json!({"type": "string", "enum": ["SUBMITTED"]});
        properties["task_state"] = json!({"type": "string", "enum": ["DRAFT"]});
        properties["result_digest"] = json!({"type": "null"});
        properties["failure_stage"] = json!({"type": "null"});
        properties["failure_code"] = json!({"type": "null"});
    }
    let mut required = vec![
        "schema_version",
        "status",
        "task_state",
        "task_ref",
        "ledger_head_digest",
        "result_digest",
        "failure_stage",
        "failure_code",
    ];
    if general {
        let object = properties.as_object_mut().expect("properties");
        object.insert(
            "objective".to_owned(),
            json!({"type":"string","minLength":1,"maxLength":512}),
        );
        object.insert("project_id".to_owned(), json!({"type":"string","minLength":2,"maxLength":64,"pattern":"^[a-z0-9][a-z0-9._-]{1,63}$"}));
        object.insert(
            "project_name".to_owned(),
            json!({"type":"string","minLength":1,"maxLength":64}),
        );
        object.insert("project_snapshot_id".to_owned(), json!({"type":"string","minLength":1,"maxLength":159,"pattern":"^[A-Za-z0-9][A-Za-z0-9._:-]{0,158}$"}));
        required.extend([
            "objective",
            "project_id",
            "project_name",
            "project_snapshot_id",
        ]);
    }
    json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false
    })
}

fn managed_task_public_output_variant() -> Value {
    let mut schema = task_public_output_variant(true);
    let properties = schema["properties"]
        .as_object_mut()
        .expect("managed status properties");
    properties.remove("objective");
    properties.insert(
        "objective_summary".to_owned(),
        json!({"type": "string", "enum": ["Objective retained; digest only."]}),
    );
    properties.insert(
        "objective_digest".to_owned(),
        json!({"type": "string", "minLength": 64, "maxLength": 64, "pattern": "^[0-9a-f]{64}$"}),
    );
    properties["schema_version"] = json!({"type": "string", "enum": ["lattice.task.status.v4"]});
    properties["status"] = json!({
        "type": "string",
        "enum": ["SUBMITTED", "RUNNING", "BLOCKED", "FAILED", "AWAITING_MERGE_APPROVAL"]
    });
    properties["task_state"] = json!({
        "type": "string",
        "enum": [
            "NOT_SUBMITTED", "DRAFT", "AWAITING_EXECUTION_APPROVAL", "PREPARING",
            "EXECUTING", "VERIFYING", "REVIEWING", "AWAITING_MERGE_APPROVAL",
            "MERGING", "COMPLETED", "REJECTED", "BLOCKED", "FAILED", "STOPPING",
            "CANCELLED"
        ]
    });
    properties["result_digest"] = json!({
        "anyOf": [
            {"type": "string", "minLength": 64, "maxLength": 64, "pattern": "^[0-9a-f]{64}$"},
            {"type": "null"}
        ]
    });
    for field in ["failure_stage", "failure_code"] {
        properties[field] = json!({
            "anyOf": [
                {"type": "string", "minLength": 1, "maxLength": 128, "pattern": "^[A-Z0-9_]+$"},
                {"type": "null"}
            ]
        });
    }
    properties.insert("worker_running".to_owned(), json!({"type": "boolean"}));
    properties.insert(
        "attempt".to_owned(),
        json!({"anyOf": [{"type": "integer", "minimum": 1, "maximum": 3}, {"type": "null"}]}),
    );
    properties.insert(
        "retry_count".to_owned(),
        json!({"type": "integer", "minimum": 0, "maximum": 2}),
    );
    properties.insert(
        "model".to_owned(),
        json!({"anyOf": [{"type": "string", "enum": ["gpt-5.6-luna", "gpt-5.6-terra", "gpt-5.6-sol"]}, {"type": "null"}]}),
    );
    properties.insert(
        "reasoning".to_owned(),
        json!({"anyOf": [{"type": "string", "enum": ["low", "medium", "high", "xhigh", "max", "ultra"]}, {"type": "null"}]}),
    );
    let identifier = json!({
        "anyOf": [
            {"type": "string", "minLength": 1, "maxLength": 256, "pattern": "^[A-Za-z0-9][A-Za-z0-9._:-]{0,255}$"},
            {"type": "null"}
        ]
    });
    properties.insert("thread_id".to_owned(), identifier.clone());
    properties.insert("turn_id".to_owned(), identifier);
    properties.insert(
        "last_progress_at".to_owned(),
        json!({"anyOf": [{"type": "string", "format": "date-time", "pattern": "Z$"}, {"type": "null"}]}),
    );
    properties.insert(
        "blocker".to_owned(),
        json!({"anyOf": [{"type": "string", "minLength": 1, "maxLength": 128, "pattern": "^[A-Z0-9_]+$"}, {"type": "null"}]}),
    );
    properties.insert(
        "verification_status".to_owned(),
        json!({"type": "string", "enum": ["NOT_STARTED", "RUNNING", "PASSED", "FAILED"]}),
    );
    let nullable_sha = json!({
        "anyOf": [
            {"type": "string", "minLength": 64, "maxLength": 64, "pattern": "^[0-9a-f]{64}$"},
            {"type": "null"}
        ]
    });
    properties.insert("verification_digest".to_owned(), nullable_sha.clone());
    properties.insert("evidence_digest".to_owned(), nullable_sha);
    properties.insert(
        "resource_observation".to_owned(),
        json!({
            "anyOf": [
                {
                    "type": "object",
                    "properties": {
                        "scope": {"type": "string", "enum": ["TASK_CUMULATIVE"]},
                        "attempts_observed": {"type": "integer", "minimum": 0},
                        "model_calls": {"type": "integer", "minimum": 0},
                        "remaining_model_calls": {"type": "integer", "minimum": 0},
                        "remaining_total_tokens": nullable_non_negative_integer_schema(),
                        "input_tokens": nullable_non_negative_integer_schema(),
                        "cached_input_tokens": nullable_non_negative_integer_schema(),
                        "output_tokens": nullable_non_negative_integer_schema(),
                        "reasoning_output_tokens": nullable_non_negative_integer_schema(),
                        "total_tokens": nullable_non_negative_integer_schema(),
                        "external_cost_status": {"type": "string", "enum": ["UNAVAILABLE"]}
                    },
                    "required": [
                        "scope", "attempts_observed", "model_calls",
                        "remaining_model_calls", "remaining_total_tokens",
                        "input_tokens", "cached_input_tokens", "output_tokens",
                        "reasoning_output_tokens", "total_tokens", "external_cost_status"
                    ],
                    "additionalProperties": false
                },
                {"type": "null"}
            ]
        }),
    );
    properties.insert(
        "next_action".to_owned(),
        json!({"type": "string", "minLength": 1, "maxLength": 256}),
    );
    properties.insert(
        "foreman_generation".to_owned(),
        json!({"type": "integer", "minimum": 1}),
    );
    properties.insert(
        "foreman_checkpoint_digest".to_owned(),
        json!({"type": "string", "minLength": 64, "maxLength": 64, "pattern": "^[0-9a-f]{64}$"}),
    );

    let required = schema["required"]
        .as_array_mut()
        .expect("managed required fields");
    required.retain(|field| field.as_str() != Some("objective"));
    required.extend(
        [
            "objective_summary",
            "objective_digest",
            "worker_running",
            "attempt",
            "retry_count",
            "model",
            "reasoning",
            "thread_id",
            "turn_id",
            "last_progress_at",
            "blocker",
            "verification_status",
            "verification_digest",
            "evidence_digest",
            "resource_observation",
            "next_action",
            "foreman_generation",
            "foreman_checkpoint_digest",
        ]
        .into_iter()
        .map(|field| json!(field)),
    );
    schema
}

fn nullable_non_negative_integer_schema() -> Value {
    json!({
        "anyOf": [
            {"type": "integer", "minimum": 0},
            {"type": "null"}
        ]
    })
}

fn client_request_id_input_schema() -> Value {
    json!({
        "type": "string",
        "minLength": 1,
        "maxLength": 64,
        "pattern": "^[A-Za-z0-9][A-Za-z0-9._:-]{0,63}$",
        "description": "Bounded ASCII idempotency key without recognized secret material."
    })
}

fn general_task_input_variant(field: &str, excludes_canary: bool) -> Value {
    let mut schema = json!({
        "type": "object",
        "properties": {
            "client_request_id": client_request_id_input_schema(),
            (field): {
                "type": "string",
                "minLength": 1,
                "maxLength": 512,
                "description": "NFC text without leading/trailing whitespace, control characters, or secret material."
            },
            "project_id": {
                "type": "string",
                "minLength": 2,
                "maxLength": 64,
                "pattern": "^[a-z0-9][a-z0-9._-]{1,63}$"
            },
            "project_name": {
                "type": "string",
                "minLength": 1,
                "maxLength": 64,
                "description": "Exact NFC Control catalog display name."
            }
        },
        "required": ["client_request_id", field],
        "not": {"required": ["project_id", "project_name"]},
        "additionalProperties": false
    });
    if excludes_canary {
        schema["properties"][field]["not"] = json!({"enum": [CONTROLLED_CODEX_CANARY]});
    }
    schema
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
            "lattice_delivery_reconcile",
            "lattice_foreman_checkpoint"
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
    assert_eq!(rejected["result"]["isError"], true);
    assert_eq!(
        rejected["result"]["structuredContent"]["code"],
        "LATTICE_MCP_SESSION_EXHAUSTED"
    );
    assert_eq!(
        rejected["result"]["structuredContent"]["effect_started"],
        false
    );
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
        Some(7)
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
fn tool_list_is_exactly_seven_bounded_tools_with_closed_schemas() {
    let (mut server, _, _) = server();
    initialize(&mut server);

    let response = server
        .handle(json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}))
        .expect("tool list");
    let tools = response["result"]["tools"].as_array().expect("tools");

    assert_eq!(tools.len(), 7);
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
            "lattice_delivery_reconcile",
            "lattice_foreman_checkpoint"
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
            "oneOf": [
                {
                    "type": "object",
                    "properties": {
                        "client_request_id": client_request_id_input_schema(),
                        "intent": {
                            "type": "string",
                            "enum": [CONTROLLED_CODEX_CANARY]
                        }
                    },
                    "required": ["client_request_id", "intent"],
                    "additionalProperties": false
                },
                general_task_input_variant("objective", false),
                general_task_input_variant("intent", true)
            ]
        })
    );
    assert_eq!(
        tools[3]["inputSchema"],
        json!({
            "type": "object",
            "properties": {
                "client_request_id": client_request_id_input_schema(),
                "task_ref": {
                    "type": "string",
                    "minLength": 64,
                    "maxLength": 64,
                    "pattern": "^[0-9a-f]{64}$"
                }
            },
            "required": ["task_ref"],
            "additionalProperties": false
        })
    );
    assert_eq!(tools[2]["outputSchema"], task_output_schema);
    assert_eq!(tools[3]["outputSchema"], task_output_schema);
    assert_eq!(tools[6]["inputSchema"]["additionalProperties"], false);
    assert_eq!(
        tools[6]["inputSchema"]["required"].as_array().map(Vec::len),
        Some(7)
    );
    assert_eq!(tools[6]["outputSchema"]["additionalProperties"], false);
    assert!(tools[2].get("annotations").is_none());
    assert!(tools[3].get("annotations").is_none());
}

#[test]
fn foreman_checkpoint_rejects_prohibited_fields_before_dispatch() {
    for property in [
        "worker",
        "thread",
        "task",
        "branch",
        "worktree",
        "head",
        "authority",
        "lease",
        "fence",
        "db",
        "sql",
        "path",
        "command",
    ] {
        let calls = Rc::new(Cell::new(0));
        let mut server = McpServer::new(
            CheckpointService {
                calls: calls.clone(),
            },
            fixed_binding().clone(),
        );
        initialize(&mut server);
        let mut arguments = valid_foreman_checkpoint_arguments()
            .as_object()
            .expect("arguments")
            .clone();
        arguments.insert(property.to_owned(), json!("forbidden"));
        let response = server
            .handle(json!({
                "jsonrpc": "2.0",
                "id": property,
                "method": "tools/call",
                "params": {
                    "name": "lattice_foreman_checkpoint",
                    "arguments": arguments
                }
            }))
            .expect("rejection");
        assert_eq!(response["error"]["code"], -32602, "{property}");
        assert_eq!(
            response["error"]["data"]["code"], "FOREMAN_CHECKPOINT_INVALID",
            "{property}"
        );
        assert_eq!(calls.get(), 0, "{property}");
    }
}

#[test]
fn foreman_checkpoint_invalid_format_time_and_blocker_matrix_has_stable_protocol_code() {
    let cases = [
        ("unsafe-id", "checkpoint_id", json!("-bad")),
        ("zero-generation", "generation", json!(0)),
        (
            "offset-time",
            "occurred_at",
            json!("2026-08-25T00:00:01+00:00"),
        ),
        (
            "fraction-time",
            "occurred_at",
            json!("2026-08-25T00:00:01.000Z"),
        ),
        ("invalid-date", "occurred_at", json!("2026-99-99T00:00:01Z")),
        ("unknown-state", "state", json!("active")),
        (
            "uppercase-heartbeat",
            "heartbeat_ref",
            json!(format!("heartbeat:sha256:{}", "A".repeat(64))),
        ),
        (
            "bad-evidence-prefix",
            "evidence_ref",
            json!(format!("heartbeat:sha256:{}", "b".repeat(64))),
        ),
    ];
    for (id, field, value) in cases {
        let calls = Rc::new(Cell::new(0));
        let mut server = McpServer::new(
            CheckpointService {
                calls: calls.clone(),
            },
            fixed_binding().clone(),
        );
        initialize(&mut server);
        let mut arguments = valid_foreman_checkpoint_arguments()
            .as_object()
            .expect("arguments")
            .clone();
        arguments.insert(field.to_owned(), value);
        let response = server
            .handle(json!({
                "jsonrpc":"2.0", "id":id, "method":"tools/call",
                "params":{"name":"lattice_foreman_checkpoint","arguments":arguments}
            }))
            .expect("rejection");
        assert_eq!(response["error"]["code"], -32602, "{id}");
        assert_eq!(
            response["error"]["data"]["code"], "FOREMAN_CHECKPOINT_INVALID",
            "{id}"
        );
        assert_eq!(calls.get(), 0, "{id}");
    }

    for (id, state, blocker) in [
        ("active-blocker", "ACTIVE", json!("TASK-094")),
        ("completed-blocker", "COMPLETED", json!("TASK-094")),
        ("blocked-without-blocker", "BLOCKED", Value::Null),
    ] {
        let calls = Rc::new(Cell::new(0));
        let mut server = McpServer::new(
            CheckpointService {
                calls: calls.clone(),
            },
            fixed_binding().clone(),
        );
        initialize(&mut server);
        let mut arguments = valid_foreman_checkpoint_arguments()
            .as_object()
            .expect("arguments")
            .clone();
        arguments.insert("state".to_owned(), json!(state));
        arguments.insert("blocker_ref".to_owned(), blocker);
        let response = server
            .handle(json!({
                "jsonrpc":"2.0", "id":id, "method":"tools/call",
                "params":{"name":"lattice_foreman_checkpoint","arguments":arguments}
            }))
            .expect("rejection");
        assert_eq!(response["error"]["code"], -32602, "{id}");
        assert_eq!(
            response["error"]["data"]["code"], "FOREMAN_CHECKPOINT_INVALID",
            "{id}"
        );
        assert_eq!(calls.get(), 0, "{id}");
    }
}

#[test]
fn foreman_checkpoint_preserves_a_numeric_legacy_dependency_string() {
    let calls = Rc::new(Cell::new(0));
    let mut server = McpServer::new(
        CheckpointService {
            calls: calls.clone(),
        },
        fixed_binding().clone(),
    );
    initialize(&mut server);
    let mut arguments = valid_foreman_checkpoint_arguments()
        .as_object()
        .expect("arguments")
        .clone();
    arguments.insert("state".to_owned(), json!("BLOCKED"));
    arguments.insert(
        "blocker_ref".to_owned(),
        json!(
            "dependency:v2:TASK-106:TASK-107:TASK-107-WORKTREE:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        ),
    );
    let response = server
        .handle(json!({
            "jsonrpc":"2.0", "id":"legacy-dependency", "method":"tools/call",
            "params":{"name":"lattice_foreman_checkpoint","arguments":arguments}
        }))
        .expect("legacy checkpoint response");
    assert_eq!(response["result"]["isError"], false);
    assert_eq!(calls.get(), 1);
}

#[test]
fn foreman_checkpoint_dispatches_only_valid_closed_intent() {
    let calls = Rc::new(Cell::new(0));
    let mut server = McpServer::new(
        CheckpointService {
            calls: calls.clone(),
        },
        fixed_binding().clone(),
    );
    initialize(&mut server);
    let response = server
        .handle(json!({
            "jsonrpc": "2.0",
            "id": "checkpoint",
            "method": "tools/call",
            "params": {
                "name": "lattice_foreman_checkpoint",
                "arguments": valid_foreman_checkpoint_arguments()
            }
        }))
        .expect("checkpoint response");
    assert_eq!(response["result"]["isError"], false);
    assert_eq!(
        response["result"]["structuredContent"]["status"],
        "RECORDED"
    );
    assert_eq!(calls.get(), 1);
}

#[test]
fn foreman_checkpoint_accepts_only_the_closed_dependency_object() {
    let valid_dependency = json!({
        "schema": "lattice.dependency-blocker/1.0",
        "parent_task_id": "TASK-106",
        "dependency_task_id": "TASK-107",
        "dependency_worktree_id": "TASK-107-WORKTREE",
        "dependency_branch": "lattice/task-107",
        "base_sha": "a".repeat(40),
        "next_action": "COMPLETE_DEPENDENCY"
    });
    let mut valid_arguments = valid_foreman_checkpoint_arguments()
        .as_object()
        .expect("arguments")
        .clone();
    valid_arguments.insert("state".to_owned(), json!("BLOCKED"));
    valid_arguments.insert("blocker_ref".to_owned(), valid_dependency.clone());

    let calls = Rc::new(Cell::new(0));
    let mut server = McpServer::new(
        CheckpointService {
            calls: calls.clone(),
        },
        fixed_binding().clone(),
    );
    initialize(&mut server);
    let response = server
        .handle(json!({
            "jsonrpc":"2.0", "id":"dependency-valid", "method":"tools/call",
            "params":{"name":"lattice_foreman_checkpoint","arguments":valid_arguments}
        }))
        .expect("dependency checkpoint");
    assert_eq!(response["result"]["isError"], false);
    assert_eq!(calls.get(), 1);

    for (id, invalid_evidence) in [
        ("evidence-null", Value::Null),
        ("evidence-object", json!({"digest": "a".repeat(64)})),
        (
            "evidence-prefix",
            json!(format!("secret={}", "a".repeat(64))),
        ),
        (
            "evidence-uppercase",
            json!(format!("evidence:sha256:{}", "A".repeat(64))),
        ),
    ] {
        let calls = Rc::new(Cell::new(0));
        let mut server = McpServer::new(
            CheckpointService {
                calls: calls.clone(),
            },
            fixed_binding().clone(),
        );
        initialize(&mut server);
        let mut arguments = valid_foreman_checkpoint_arguments()
            .as_object()
            .expect("arguments")
            .clone();
        arguments.insert("state".to_owned(), json!("BLOCKED"));
        arguments.insert("blocker_ref".to_owned(), valid_dependency.clone());
        arguments.insert("evidence_ref".to_owned(), invalid_evidence);
        let response = server
            .handle(json!({
                "jsonrpc":"2.0", "id":id, "method":"tools/call",
                "params":{"name":"lattice_foreman_checkpoint","arguments":arguments}
            }))
            .expect("invalid outer evidence rejection");
        assert_eq!(response["error"]["code"], -32602, "{id}");
        assert_eq!(
            response["error"]["data"]["code"], "FOREMAN_CHECKPOINT_INVALID",
            "{id}"
        );
        assert_eq!(calls.get(), 0, "{id}");
    }

    for (id, field, value) in [
        ("schema", "schema", json!("lattice.dependency-blocker/2.0")),
        ("parent", "parent_task_id", json!("task-106")),
        ("dependency", "dependency_task_id", json!("TASK/107")),
        ("worktree", "dependency_worktree_id", json!("../escape")),
        ("branch", "dependency_branch", json!("feature/task-107")),
        ("base", "base_sha", json!("A".repeat(40))),
        ("action", "next_action", json!("CONTINUE_PARENT")),
    ] {
        let calls = Rc::new(Cell::new(0));
        let mut server = McpServer::new(
            CheckpointService {
                calls: calls.clone(),
            },
            fixed_binding().clone(),
        );
        initialize(&mut server);
        let mut dependency = valid_dependency.as_object().expect("dependency").clone();
        dependency.insert(field.to_owned(), value);
        let mut arguments = valid_foreman_checkpoint_arguments()
            .as_object()
            .expect("arguments")
            .clone();
        arguments.insert("state".to_owned(), json!("BLOCKED"));
        arguments.insert("blocker_ref".to_owned(), Value::Object(dependency));
        let response = server
            .handle(json!({
                "jsonrpc":"2.0", "id":id, "method":"tools/call",
                "params":{"name":"lattice_foreman_checkpoint","arguments":arguments}
            }))
            .expect("dependency rejection");
        assert_eq!(response["error"]["code"], -32602, "{id}");
        assert_eq!(
            response["error"]["data"]["code"], "FOREMAN_CHECKPOINT_INVALID",
            "{id}"
        );
        assert_eq!(calls.get(), 0, "{id}");
    }

    let calls = Rc::new(Cell::new(0));
    let mut server = McpServer::new(
        CheckpointService {
            calls: calls.clone(),
        },
        fixed_binding().clone(),
    );
    initialize(&mut server);
    let mut extra_dependency = valid_dependency.as_object().expect("dependency").clone();
    extra_dependency.insert("path".to_owned(), json!("C:/unsafe"));
    let mut arguments = valid_foreman_checkpoint_arguments()
        .as_object()
        .expect("arguments")
        .clone();
    arguments.insert("state".to_owned(), json!("BLOCKED"));
    arguments.insert("blocker_ref".to_owned(), Value::Object(extra_dependency));
    let response = server
        .handle(json!({
            "jsonrpc":"2.0", "id":"extra", "method":"tools/call",
            "params":{"name":"lattice_foreman_checkpoint","arguments":arguments}
        }))
        .expect("extra-field rejection");
    assert_eq!(response["error"]["code"], -32602);
    assert_eq!(calls.get(), 0);
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
        (
            "legacy-checkpoint",
            "lattice_foreman_checkpoint",
            valid_foreman_checkpoint_arguments(),
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
        (
            "stateless-checkpoint",
            "lattice_foreman_checkpoint",
            valid_foreman_checkpoint_arguments(),
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
fn general_task_submit_accepts_a_bounded_objective_without_a_path_or_authority_input() {
    let (mut server, submits, statuses) = task_server();
    initialize(&mut server);

    let response = server
        .handle(json!({
            "jsonrpc": "2.0",
            "id": "general-submit",
            "method": "tools/call",
            "params": {
                "name": "lattice_task_submit",
                "arguments": {
                    "client_request_id": "general-task-001",
                    "objective": "完成角色系統",
                    "project_name": "AI 劇本"
                }
            }
        }))
        .expect("general task submit response");

    assert_eq!(response["result"]["isError"], false);
    let submits = submits.borrow();
    assert_eq!(submits.len(), 1);
    assert!(!submits[0].is_controlled_canary());
    assert_eq!(submits[0].objective(), Some("完成角色系統"));
    assert_eq!(submits[0].project_id(), None);
    assert_eq!(submits[0].project_name(), Some("AI 劇本"));
    assert!(statuses.borrow().is_empty());
}

#[test]
fn shell_sql_and_path_looking_objective_remains_inert_task_data() {
    let (mut server, submits, statuses) = task_server();
    initialize(&mut server);
    let objective = r"Review $(whoami); DROP TABLE tasks; C:\literal\objective";

    let response = server
        .handle(json!({
            "jsonrpc": "2.0",
            "id": "inert-general-objective",
            "method": "tools/call",
            "params": {
                "name": "lattice_task_submit",
                "arguments": {
                    "client_request_id": "general-inert-data-001",
                    "objective": objective,
                    "project_name": "AI 劇本"
                }
            }
        }))
        .expect("inert general objective response");

    assert_eq!(response["result"]["isError"], false);
    let submits = submits.borrow();
    assert_eq!(submits.len(), 1);
    assert_eq!(submits[0].objective(), Some(objective));
    assert_eq!(submits[0].project_name(), Some("AI 劇本"));
    assert!(statuses.borrow().is_empty());
}

#[test]
fn general_task_submit_normalizes_the_natural_intent_alias_to_the_same_typed_objective() {
    let (mut server, submits, _) = task_server();
    initialize(&mut server);

    let response = server
        .handle(json!({
            "jsonrpc": "2.0",
            "id": "general-intent-submit",
            "method": "tools/call",
            "params": {
                "name": "lattice_task_submit",
                "arguments": {
                    "client_request_id": "general-task-002",
                    "intent": "完成角色系統",
                    "project_id": "legacy-project-id"
                }
            }
        }))
        .expect("general intent submit response");

    assert_eq!(response["result"]["isError"], false);
    let submits = submits.borrow();
    assert_eq!(submits.len(), 1);
    assert_eq!(submits[0].objective(), Some("完成角色系統"));
    assert_eq!(submits[0].project_id(), Some("legacy-project-id"));
    assert_eq!(submits[0].project_name(), None);
}

#[test]
fn general_task_submit_uses_the_same_unicode_character_bounds_as_its_json_schema() {
    let (mut server, submits, _) = task_server();
    initialize(&mut server);
    let objective = "😀".repeat(512);
    let project_name = "界".repeat(64);

    let response = server
        .handle(json!({
            "jsonrpc": "2.0",
            "id": "general-unicode-boundary",
            "method": "tools/call",
            "params": {
                "name": "lattice_task_submit",
                "arguments": {
                    "client_request_id": "general-unicode-001",
                    "objective": objective,
                    "project_name": project_name
                }
            }
        }))
        .expect("general unicode submit response");

    assert_eq!(response["result"]["isError"], false);
    let submits = submits.borrow();
    assert_eq!(submits.len(), 1);
    assert_eq!(
        submits[0].objective().expect("objective").chars().count(),
        512
    );
    assert_eq!(
        submits[0]
            .project_name()
            .expect("project name")
            .chars()
            .count(),
        64
    );
}

#[test]
fn task_tools_emit_only_closed_public_status_shapes() {
    let mut maximum_snapshot = submitted_general_task_status();
    maximum_snapshot["project_snapshot_id"] = json!("a".repeat(159));
    let accepted = [
        completed_task_status(),
        submitted_general_task_status(),
        managed_general_task_status(),
        maximum_snapshot,
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
fn task_tools_emit_redacted_v5_and_reject_legacy_objective_disclosure() {
    let redacted = submitted_general_task_status();
    for tool_name in ["lattice_task_submit", "lattice_task_status"] {
        let response = call_task_tool_with_output(tool_name, redacted.clone());
        assert_eq!(response["result"]["isError"], false, "{tool_name}");
        let projection = &response["result"]["structuredContent"];
        assert_eq!(projection["schema_version"], "lattice.task.status.v5");
        assert_eq!(
            projection["objective_summary"],
            "Objective retained; digest only."
        );
        assert_eq!(projection["objective_digest"], RESULT_DIGEST);
        assert!(projection.get("objective").is_none());

        let legacy = json!({
            "schema_version": "lattice.task.status.v3",
            "status": "SUBMITTED",
            "task_state": "DRAFT",
            "task_ref": TASK_REF,
            "ledger_head_digest": LEDGER_HEAD_DIGEST,
            "result_digest": null,
            "failure_stage": null,
            "failure_code": null,
            "objective": "Internal acquisition codename Quiet Orchard",
            "project_id": "legacy-project-id",
            "project_name": "AI 劇本",
            "project_snapshot_id": "legacy-project-id:snapshot:1"
        });
        let rejected = call_task_tool_with_output(tool_name, legacy);
        assert_eq!(rejected["result"]["isError"], true, "{tool_name}");
        assert_eq!(
            rejected["result"]["structuredContent"]["code"],
            "LATTICE_TASK_PUBLIC_STATUS_REJECTED"
        );
    }
}

#[test]
fn redacted_v5_rejects_objective_or_unclosed_redaction_fields() {
    let mut invalid = Vec::new();
    let mut disclosed = submitted_general_task_status();
    disclosed["objective"] = json!("private roadmap milestone Juniper");
    invalid.push(disclosed);
    let mut summary = submitted_general_task_status();
    summary["objective_summary"] = json!("private roadmap milestone Juniper");
    invalid.push(summary);
    let mut digest = submitted_general_task_status();
    digest["objective_digest"] = json!("A".repeat(64));
    invalid.push(digest);

    for output in invalid {
        let response = call_task_tool_with_output("lattice_task_status", output);
        assert_eq!(response["result"]["isError"], true);
        assert_eq!(
            response["result"]["structuredContent"]["code"],
            "LATTICE_TASK_PUBLIC_STATUS_REJECTED"
        );
    }
}

#[test]
fn managed_task_status_v4_accepts_closed_phases_and_nullable_worker_evidence() {
    let mut nullable = managed_general_task_status();
    nullable["status"] = json!("SUBMITTED");
    nullable["task_state"] = json!("AWAITING_EXECUTION_APPROVAL");
    nullable["worker_running"] = json!(false);
    for field in [
        "attempt",
        "model",
        "reasoning",
        "thread_id",
        "turn_id",
        "last_progress_at",
        "blocker",
        "verification_digest",
        "evidence_digest",
        "resource_observation",
    ] {
        nullable[field] = Value::Null;
    }
    nullable["retry_count"] = json!(0);
    nullable["verification_status"] = json!("NOT_STARTED");
    nullable["next_action"] = json!("Approve bounded local execution.");

    let mut accepted = vec![managed_general_task_status(), nullable];
    for (status, state) in [
        ("SUBMITTED", "DRAFT"),
        ("RUNNING", "VERIFYING"),
        ("BLOCKED", "BLOCKED"),
        ("FAILED", "FAILED"),
        ("AWAITING_MERGE_APPROVAL", "AWAITING_MERGE_APPROVAL"),
    ] {
        let mut value = managed_general_task_status();
        value["status"] = json!(status);
        value["task_state"] = json!(state);
        accepted.push(value);
    }

    for tool_name in ["lattice_task_submit", "lattice_task_status"] {
        for output in &accepted {
            let response = call_task_tool_with_output(tool_name, output.clone());
            assert_eq!(
                response["result"]["isError"], false,
                "{tool_name}: {output}"
            );
            assert_eq!(
                response["result"]["structuredContent"], *output,
                "{tool_name}: {output}"
            );
        }
    }
}

#[test]
fn managed_task_status_v4_rejects_non_closed_or_unsafe_projection_fields() {
    let replace = |field: &str, replacement: Value| {
        let mut value = managed_general_task_status();
        value[field] = replacement;
        value
    };
    let mut invalid = Vec::<(String, Value)>::new();
    for field in [
        "objective_summary",
        "objective_digest",
        "worker_running",
        "attempt",
        "retry_count",
        "model",
        "reasoning",
        "thread_id",
        "turn_id",
        "last_progress_at",
        "blocker",
        "verification_status",
        "verification_digest",
        "evidence_digest",
        "resource_observation",
        "next_action",
        "foreman_generation",
        "foreman_checkpoint_digest",
    ] {
        let mut value = managed_general_task_status();
        value
            .as_object_mut()
            .expect("managed status object")
            .remove(field);
        invalid.push((format!("missing-{field}"), value));
    }
    for (label, field, replacement) in [
        ("schema", "schema_version", json!("lattice.task.status.v5")),
        ("status", "status", json!("COMPLETED")),
        ("task-state", "task_state", json!("UNKNOWN")),
        ("objective-summary-empty", "objective_summary", json!("")),
        (
            "objective-summary-not-fixed",
            "objective_summary",
            json!("Apply one bounded local change"),
        ),
        (
            "objective-summary-secret",
            "objective_summary",
            json!("clone https://alice:hunter2@example.invalid/repo"),
        ),
        (
            "objective-digest",
            "objective_digest",
            json!("A".repeat(64)),
        ),
        ("running-type", "worker_running", json!("true")),
        ("attempt-zero", "attempt", json!(0)),
        ("attempt-four", "attempt", json!(4)),
        ("attempt-fraction", "attempt", json!(1.5)),
        ("retry-negative", "retry_count", json!(-1)),
        ("retry-three", "retry_count", json!(3)),
        ("model", "model", json!("gpt-5.5")),
        ("reasoning", "reasoning", json!("extreme")),
        ("thread-empty", "thread_id", json!("")),
        ("thread-path", "thread_id", json!("thread/../../secret")),
        ("thread-long", "thread_id", json!("a".repeat(257))),
        ("turn-control", "turn_id", json!("turn\nsecret")),
        (
            "progress-offset",
            "last_progress_at",
            json!("2026-08-26T20:34:56+08:00"),
        ),
        (
            "progress-invalid",
            "last_progress_at",
            json!("2026-99-99T12:34:56Z"),
        ),
        ("blocker-lower", "blocker", json!("heartbeat_timeout")),
        ("blocker-long", "blocker", json!("A".repeat(129))),
        (
            "verification-status",
            "verification_status",
            json!("COMPLETE"),
        ),
        (
            "verification-digest",
            "verification_digest",
            json!("A".repeat(64)),
        ),
        ("evidence-digest", "evidence_digest", json!("abc")),
        ("next-empty", "next_action", json!("")),
        ("next-untrimmed", "next_action", json!(" wait")),
        ("next-control", "next_action", json!("wait\nnow")),
        ("next-long", "next_action", json!("界".repeat(257))),
        (
            "next-secret",
            "next_action",
            json!("authorization: Bearer do-not-echo"),
        ),
        ("generation-zero", "foreman_generation", json!(0)),
        ("generation-fraction", "foreman_generation", json!(1.5)),
        (
            "checkpoint-digest",
            "foreman_checkpoint_digest",
            json!("A".repeat(64)),
        ),
    ] {
        invalid.push((label.to_owned(), replace(field, replacement)));
    }

    for (label, resource) in [
        (
            "resource-negative",
            json!({
                "scope": "TASK_CUMULATIVE",
                "attempts_observed": 1,
                "model_calls": 1,
                "remaining_model_calls": 5,
                "remaining_total_tokens": 99840,
                "input_tokens": -1,
                "cached_input_tokens": null,
                "output_tokens": null,
                "reasoning_output_tokens": null,
                "total_tokens": null,
                "external_cost_status": "UNAVAILABLE"
            }),
        ),
        (
            "resource-fraction",
            json!({
                "scope": "TASK_CUMULATIVE",
                "attempts_observed": 1,
                "model_calls": 1,
                "remaining_model_calls": 5,
                "remaining_total_tokens": 99840,
                "input_tokens": 1.5,
                "cached_input_tokens": null,
                "output_tokens": null,
                "reasoning_output_tokens": null,
                "total_tokens": null,
                "external_cost_status": "UNAVAILABLE"
            }),
        ),
        (
            "resource-cost-status",
            json!({
                "scope": "TASK_CUMULATIVE",
                "attempts_observed": 1,
                "model_calls": 1,
                "remaining_model_calls": 5,
                "remaining_total_tokens": 99840,
                "input_tokens": null,
                "cached_input_tokens": null,
                "output_tokens": null,
                "reasoning_output_tokens": null,
                "total_tokens": null,
                "external_cost_status": "AVAILABLE"
            }),
        ),
        (
            "resource-extra",
            json!({
                "scope": "TASK_CUMULATIVE",
                "attempts_observed": 1,
                "model_calls": 1,
                "remaining_model_calls": 5,
                "remaining_total_tokens": 99840,
                "input_tokens": null,
                "cached_input_tokens": null,
                "output_tokens": null,
                "reasoning_output_tokens": null,
                "total_tokens": null,
                "external_cost_status": "UNAVAILABLE",
                "cost_micros": 0
            }),
        ),
        (
            "resource-missing",
            json!({
                "scope": "TASK_CUMULATIVE",
                "attempts_observed": 1,
                "model_calls": 1,
                "remaining_model_calls": 5,
                "remaining_total_tokens": 99840,
                "input_tokens": null,
                "cached_input_tokens": null,
                "output_tokens": null,
                "reasoning_output_tokens": null,
                "external_cost_status": "UNAVAILABLE"
            }),
        ),
        (
            "resource-scope",
            json!({
                "scope": "LATEST_ATTEMPT",
                "attempts_observed": 1,
                "model_calls": 1,
                "remaining_model_calls": 5,
                "remaining_total_tokens": 99840,
                "input_tokens": null,
                "cached_input_tokens": null,
                "output_tokens": null,
                "reasoning_output_tokens": null,
                "total_tokens": null,
                "external_cost_status": "UNAVAILABLE"
            }),
        ),
        (
            "resource-negative-model-calls",
            json!({
                "scope": "TASK_CUMULATIVE",
                "attempts_observed": 1,
                "model_calls": -1,
                "remaining_model_calls": 5,
                "remaining_total_tokens": null,
                "input_tokens": null,
                "cached_input_tokens": null,
                "output_tokens": null,
                "reasoning_output_tokens": null,
                "total_tokens": null,
                "external_cost_status": "UNAVAILABLE"
            }),
        ),
    ] {
        invalid.push((label.to_owned(), replace("resource_observation", resource)));
    }
    let mut extra = managed_general_task_status();
    extra
        .as_object_mut()
        .expect("managed status object")
        .insert("path".to_owned(), json!(r"C:\secret"));
    invalid.push(("extra-field".to_owned(), extra));
    let mut leaked_objective = managed_general_task_status();
    leaked_objective
        .as_object_mut()
        .expect("managed status object")
        .insert(
            "objective".to_owned(),
            json!("clone https://alice:hunter2@example.invalid/repo"),
        );
    invalid.push(("managed-objective-disclosure".to_owned(), leaked_objective));

    for tool_name in ["lattice_task_submit", "lattice_task_status"] {
        for (label, output) in &invalid {
            let response = call_task_tool_with_output(tool_name, output.clone());
            assert_eq!(
                response["result"]["isError"], true,
                "{tool_name}/{label}: {output}"
            );
            assert_eq!(
                response["result"]["structuredContent"],
                json!({
                    "status": "ERROR",
                    "code": "LATTICE_TASK_PUBLIC_STATUS_REJECTED"
                }),
                "{tool_name}/{label}: {output}"
            );
        }
    }
}

#[test]
fn task_status_accepts_task_ref_only_for_durable_general_lookup() {
    let (mut server, _, statuses) = task_server();
    initialize(&mut server);
    let response = server
        .handle(json!({
            "jsonrpc": "2.0",
            "id": "status-by-ref",
            "method": "tools/call",
            "params": {
                "name": "lattice_task_status",
                "arguments": {"task_ref": TASK_REF}
            }
        }))
        .expect("task status by ref");
    assert_eq!(response["result"]["isError"], false);
    let statuses = statuses.borrow();
    assert_eq!(statuses.len(), 1);
    assert_eq!(statuses[0].task_ref(), TASK_REF);
    assert_eq!(statuses[0].client_request_id(), None);
}

#[test]
fn task_tools_fail_closed_before_serializing_invalid_public_status() {
    let mut invalid = vec![Value::Null, json!([]), json!({})];

    let mut transitioned_intake = submitted_general_task_status();
    transitioned_intake["status"] = json!("COMPLETED");
    transitioned_intake["task_state"] = json!("COMPLETED");
    transitioned_intake["result_digest"] = json!(RESULT_DIGEST);
    invalid.push(transitioned_intake);

    let mut failed_intake = submitted_general_task_status();
    failed_intake["status"] = json!("FAILED");
    failed_intake["task_state"] = json!("FAILED");
    failed_intake["failure_stage"] = json!("CODEX");
    failed_intake["failure_code"] = json!("FORBIDDEN_EXECUTION");
    invalid.push(failed_intake);

    let mut oversized_snapshot = submitted_general_task_status();
    oversized_snapshot["project_snapshot_id"] = json!("a".repeat(160));
    invalid.push(oversized_snapshot);

    let mut invalid_project_id = submitted_general_task_status();
    invalid_project_id["project_id"] = json!("x");
    invalid.push(invalid_project_id);

    let mut secret_project_id = submitted_general_task_status();
    secret_project_id["project_id"] = json!("github_pat_do-not-echo");
    invalid.push(secret_project_id);

    let mut secret_project_snapshot = submitted_general_task_status();
    secret_project_snapshot["project_snapshot_id"] = json!("AKIAIOSFODNN7EXAMPLE");
    invalid.push(secret_project_snapshot);

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
        json!({"client_request_id": "sk-do-not-use", "intent": CONTROLLED_CODEX_CANARY}),
        json!({"client_request_id": "xghp_do-not-use", "objective": "valid"}),
        json!({"client_request_id": "token:do-not-use", "objective": "valid"}),
        json!({"client_request_id": 7, "intent": CONTROLLED_CODEX_CANARY}),
        json!({"client_request_id": CLIENT_REQUEST_ID, "intent": false}),
        json!({"client_request_id": CLIENT_REQUEST_ID, "objective": ""}),
        json!({"client_request_id": CLIENT_REQUEST_ID, "objective": "   "}),
        json!({"client_request_id": CLIENT_REQUEST_ID, "objective": " leading"}),
        json!({"client_request_id": CLIENT_REQUEST_ID, "objective": "trailing "}),
        json!({"client_request_id": CLIENT_REQUEST_ID, "objective": "line\nbreak"}),
        json!({"client_request_id": CLIENT_REQUEST_ID, "objective": "nul\0byte"}),
        json!({"client_request_id": CLIENT_REQUEST_ID, "objective": "cafe\u{301}"}),
        json!({"client_request_id": CLIENT_REQUEST_ID, "objective": "x".repeat(513)}),
        json!({"client_request_id": CLIENT_REQUEST_ID, "objective": "😀".repeat(513)}),
        json!({"client_request_id": CLIENT_REQUEST_ID, "objective": "use bearer secret-value"}),
        json!({"client_request_id": CLIENT_REQUEST_ID, "objective": "password=do-not-store"}),
        json!({"client_request_id": CLIENT_REQUEST_ID, "objective": "完成設定 secret=hunter2"}),
        json!({"client_request_id": CLIENT_REQUEST_ID, "objective": "credential: do-not-store"}),
        json!({"client_request_id": CLIENT_REQUEST_ID, "objective": "Cookie = session-value"}),
        json!({"client_request_id": CLIENT_REQUEST_ID, "objective": r#"{"password":"hunter2"}"#}),
        json!({"client_request_id": CLIENT_REQUEST_ID, "objective": r#"{"api_key":"do-not-store"}"#}),
        json!({"client_request_id": CLIENT_REQUEST_ID, "objective": "password\u{2003}=hunter2"}),
        json!({"client_request_id": CLIENT_REQUEST_ID, "objective": "api_key\u{a0}:do-not-store"}),
        json!({"client_request_id": CLIENT_REQUEST_ID, "objective": "Kpassword=do-not-store"}),
        json!({"client_request_id": CLIENT_REQUEST_ID, "objective": "Ksk-do-not-store"}),
        json!({"client_request_id": CLIENT_REQUEST_ID, "objective": "private key----- marker before -----begin marker"}),
        json!({"client_request_id": CLIENT_REQUEST_ID, "objective": "access_key=AKIAIOSFODNN7EXAMPLE"}),
        json!({"client_request_id": CLIENT_REQUEST_ID, "objective": "使用 AKIAIOSFODNN7EXAMPLE 完成設定"}),
        json!({"client_request_id": CLIENT_REQUEST_ID, "objective": "clone https://alice:hunter2@example.invalid/repo"}),
        json!({"client_request_id": CLIENT_REQUEST_ID, "objective": "fetch http://alice%3Ahunter2@example.invalid/repo"}),
        json!({"client_request_id": CLIENT_REQUEST_ID, "objective": "inspect ssh://git:private@example.invalid/repo"}),
        json!({"client_request_id": CLIENT_REQUEST_ID, "objective": "inspect ssh://git@example.invalid/repo"}),
        json!({"client_request_id": CLIENT_REQUEST_ID, "objective": "valid", "project_id": "not/a/project"}),
        json!({"client_request_id": CLIENT_REQUEST_ID, "objective": "valid", "project_id": "sk-do-not-use"}),
        json!({"client_request_id": CLIENT_REQUEST_ID, "objective": "valid", "project_name": " AI 劇本"}),
        json!({"client_request_id": CLIENT_REQUEST_ID, "objective": "valid", "project_name": "界".repeat(65)}),
        json!({"client_request_id": CLIENT_REQUEST_ID, "objective": "valid", "project_id": "5fbaf1af-dcf8-42fb-8327-ea3bcd7c580f", "project_name": "AI 劇本"}),
        json!({"client_request_id": CLIENT_REQUEST_ID, "objective": "valid", "intent": "also-valid"}),
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
        let (mut server, submits, _) = task_server();
        initialize(&mut server);
        let response = server
            .handle(json!({
                "jsonrpc": "2.0",
                "id": format!("general-{property}"),
                "method": "tools/call",
                "params": {
                    "name": "lattice_task_submit",
                    "arguments": {
                        "client_request_id": CLIENT_REQUEST_ID,
                        "objective": "完成角色系統",
                        "project_name": "AI 劇本",
                        (property): "forbidden"
                    }
                }
            }))
            .expect("dangerous general submit response");
        assert_eq!(response["error"]["code"], -32602, "{property}");
        assert!(submits.borrow().is_empty(), "{property}");
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
        json!({"task_ref": TASK_REF, "client_request_id": "sk-do-not-use"}),
        json!({"task_ref": TASK_REF, "client_request_id": "xghp_do-not-use"}),
        json!({"task_ref": TASK_REF, "client_request_id": "token:do-not-use"}),
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
    assert_eq!(list["result"]["tools"].as_array().map(Vec::len), Some(7));

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
    assert_eq!(rejected["result"]["isError"], true);
    assert_eq!(
        rejected["result"]["structuredContent"]["code"],
        "LATTICE_MCP_SESSION_EXHAUSTED"
    );
    assert_eq!(run_calls.get(), 0);
    assert_eq!(
        status_calls.get() as usize,
        MAX_TOOL_INVOCATIONS_PER_SESSION
    );
}

#[test]
fn execution_budget_preserves_a_structured_read_only_handoff_reserve() {
    let (mut server, run_calls, status_calls) = server();
    initialize(&mut server);

    for id in 0..(MAX_TOOL_INVOCATIONS_PER_SESSION - 8) {
        let response = server
            .handle(json!({
                "jsonrpc":"2.0",
                "id":id + 1000,
                "method":"tools/call",
                "params":{"name":"lattice_delivery_run","arguments":{}}
            }))
            .expect("execution response within budget");
        assert_eq!(response["result"]["isError"], false, "{id}");
    }

    let denied_execution = server
        .handle(json!({
            "jsonrpc":"2.0",
            "id":"budget-handoff",
            "method":"tools/call",
            "params":{"name":"lattice_delivery_run","arguments":{}}
        }))
        .expect("budget handoff response");
    let receipt = &denied_execution["result"]["structuredContent"];
    assert_eq!(denied_execution["result"]["isError"], true);
    assert_eq!(receipt["schema_version"], "lattice.mcp.handoff.v1");
    assert_eq!(receipt["code"], "LATTICE_MCP_BUDGET_HANDOFF_REQUIRED");
    assert_eq!(receipt["effect_started"], false);
    assert_eq!(receipt["retry_allowed"], false);
    assert_eq!(receipt["remaining_read_only_calls"], 8);
    assert_eq!(receipt["can_do"][0], "lattice_runtime_status");
    assert_eq!(
        receipt["cannot_do"],
        json!(["lattice_delivery_run", "lattice_task_submit"])
    );
    assert_eq!(
        run_calls.get() as usize,
        MAX_TOOL_INVOCATIONS_PER_SESSION - 8
    );

    for id in 0..8 {
        let response = server
            .handle(json!({
                "jsonrpc":"2.0",
                "id":id + 2000,
                "method":"tools/call",
                "params":{"name":"lattice_delivery_status","arguments":{}}
            }))
            .expect("read-only handoff response");
        assert_eq!(response["result"]["isError"], false, "{id}");
    }
    assert_eq!(status_calls.get(), 8);
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
