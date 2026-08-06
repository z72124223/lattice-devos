use std::path::Path;

use lattice_codex_adapter::{
    AppServerProtocol, AppServerSession, ProtocolError, SessionError, SessionRequest, TurnOutcome,
    TurnStatus,
};
use serde_json::{Value, json};

fn completed_exec(id: &str) -> Value {
    json!({
        "id": id,
        "type": "dynamicToolCall",
        "tool": "exec",
        "arguments": {},
        "status": "completed",
        "success": true,
        "contentItems": [{
            "type": "inputText",
            "text": "Script completed\nExit code: 0"
        }]
    })
}

fn completed_command_execution(id: &str, status: &str) -> Value {
    json!({
        "id": id,
        "type": "commandExecution",
        "command": "code-mode nested tools.shell_command",
        "cwd": "C:\\workspace",
        "status": status,
        "commandActions": [],
        "aggregatedOutput": null,
        "exitCode": 0,
        "content": null
    })
}

fn official_completed_terminal(items_view: &str) -> Value {
    let items = match items_view {
        "summary" => json!([{
            "id": "agent-final",
            "type": "agentMessage",
            "text": "Delivery complete."
        }]),
        "notLoaded" => json!([]),
        other => panic!("unsupported test itemsView: {other}"),
    };
    json!({
        "method": "turn/completed",
        "params": {
            "threadId": "thr_123",
            "turn": {
                "id": "turn_456",
                "items": items,
                "itemsView": items_view,
                "status": "completed",
                "error": null
            }
        }
    })
}

fn completed_session(
    completed_items: Vec<Value>,
    terminal: Value,
) -> Result<Option<TurnOutcome>, SessionError> {
    let mut session = AppServerSession::new();
    for request in [
        SessionRequest::Initialize,
        SessionRequest::ThreadStart,
        SessionRequest::TurnStart,
    ] {
        session.mark_request_sent(request)?;
    }
    session.ingest(json!({
        "id": 0,
        "result": {
            "userAgent": "codex_cli_rs/0.146.0",
            "platformFamily": "windows",
            "platformOs": "windows",
            "codexHome": r"C:\lattice\codex-home"
        }
    }))?;
    session.ingest(json!({"id": 1, "result": {"thread": {"id": "thr_123"}}}))?;
    session.ingest(json!({"id": 2, "result": {"turn": {"id": "turn_456"}}}))?;
    for (index, item) in completed_items.into_iter().enumerate() {
        session.ingest(json!({
            "method": "item/completed",
            "params": {
                "threadId": "thr_123",
                "turnId": "turn_456",
                "item": item,
                "completedAtMs": index
            }
        }))?;
    }
    session.ingest(terminal)
}

#[test]
fn builds_the_stable_initialize_thread_and_turn_sequence() {
    let protocol = AppServerProtocol::new("lattice_devos", "0.1.0");

    assert_eq!(
        protocol.initialize_request(0),
        json!({
            "method": "initialize",
            "id": 0,
            "params": {
                "clientInfo": {
                    "name": "lattice_devos",
                    "title": "LATTICE DevOS",
                    "version": "0.1.0"
                }
            }
        })
    );
    assert_eq!(
        protocol.initialized_notification(),
        json!({"method": "initialized"})
    );
    assert_eq!(
        protocol.thread_start_request(1, Path::new(r"C:\work\fixture")),
        json!({
            "method": "thread/start",
            "id": 1,
            "params": {
                "cwd": r"C:\work\fixture",
                "approvalPolicy": "never",
                "sandbox": "workspace-write",
                "serviceName": "lattice_devos"
            }
        })
    );
    assert_eq!(
        protocol.turn_start_request(
            2,
            "thr_123",
            Path::new(r"C:\work\fixture"),
            "Create answer.txt"
        ),
        json!({
            "method": "turn/start",
            "id": 2,
            "params": {
                "threadId": "thr_123",
                "input": [{"type": "text", "text": "Create answer.txt"}],
                "cwd": r"C:\work\fixture",
                "approvalPolicy": "never",
                "sandboxPolicy": {
                    "type": "workspaceWrite",
                    "writableRoots": [r"C:\work\fixture"],
                    "networkAccess": false
                }
            }
        })
    );
}

#[test]
fn accepts_only_a_completed_terminal_for_the_bound_turn() {
    let outcome = completed_session(
        vec![completed_exec("tool_apply"), completed_exec("tool_verify")],
        official_completed_terminal("summary"),
    )
    .expect("the exact completed turn should be accepted");

    assert_eq!(
        outcome,
        Some(TurnOutcome {
            turn_id: "turn_456".to_owned(),
            status: TurnStatus::Completed,
            error_message: None,
        })
    );

    assert_eq!(
        AppServerProtocol::parse_turn_completed(
            &json!({
                "method": "turn/completed",
                "params": {
                    "threadId": "thr_123",
                    "turn": {"id": "other", "items": [], "status": "completed"}
                }
            }),
            "thr_123",
            "turn_456"
        ),
        Err(ProtocolError::UnexpectedTurn)
    );

    assert_eq!(
        AppServerProtocol::parse_turn_completed(
            &json!({
                "method": "turn/completed",
                "params": {
                    "threadId": "other",
                    "turn": {"id": "turn_456", "items": [], "status": "completed"}
                }
            }),
            "thr_123",
            "turn_456"
        ),
        Err(ProtocolError::UnexpectedThread)
    );
}

#[test]
fn preserves_failed_and_interrupted_terminal_outcomes() {
    let failed = AppServerProtocol::parse_turn_completed(
        &json!({
            "method": "turn/completed",
            "params": {
                "threadId": "thr_123",
                "turn": {
                    "id": "turn_1",
                    "items": [],
                    "status": "failed",
                    "error": {"message": "model unavailable"}
                }
            }
        }),
        "thr_123",
        "turn_1",
    )
    .expect("a typed failed terminal remains observable")
    .expect("this is a terminal notification");
    assert_eq!(failed.status, TurnStatus::Failed);
    assert_eq!(failed.error_message.as_deref(), Some("model unavailable"));

    let interrupted = AppServerProtocol::parse_turn_completed(
        &json!({
            "method": "turn/completed",
            "params": {
                "threadId": "thr_123",
                "turn": {"id": "turn_2", "items": [], "status": "interrupted"}
            }
        }),
        "thr_123",
        "turn_2",
    )
    .expect("a typed interrupted terminal remains observable")
    .expect("this is a terminal notification");
    assert_eq!(interrupted.status, TurnStatus::Interrupted);
}

#[test]
fn ignores_non_terminal_notifications_and_rejects_malformed_terminals() {
    assert_eq!(
        AppServerProtocol::parse_turn_completed(
            &json!({"method": "item/completed", "params": {"item": {"id": "item_1"}}}),
            "thr_123",
            "turn_1"
        )
        .expect("unrelated valid notifications are ignored"),
        None
    );

    assert_eq!(
        AppServerProtocol::parse_turn_completed(
            &json!({
                "method": "turn/completed",
                "params": {
                    "threadId": "thr_123",
                    "turn": {"id": "turn_1", "items": []}
                }
            }),
            "thr_123",
            "turn_1"
        ),
        Err(ProtocolError::MalformedTerminal)
    );
}

#[test]
fn requires_the_pinned_turn_items_array_for_terminal_evidence() {
    assert_eq!(
        AppServerProtocol::parse_turn_completed(
            &json!({
                "method": "turn/completed",
                "params": {
                    "threadId": "thr_123",
                    "turn": {"id": "turn_1", "status": "completed", "error": null}
                }
            }),
            "thr_123",
            "turn_1"
        ),
        Err(ProtocolError::MalformedTerminal)
    );

    assert_eq!(
        AppServerProtocol::parse_turn_completed(
            &json!({
                "method": "turn/completed",
                "params": {
                    "threadId": "thr_123",
                    "turn": {
                        "id": "turn_1",
                        "items": {},
                        "status": "completed",
                        "error": null
                    }
                }
            }),
            "thr_123",
            "turn_1"
        ),
        Err(ProtocolError::MalformedTerminal)
    );
}

#[test]
fn enforces_the_pinned_turn_status_and_error_shape() {
    for (status, error) in [
        (
            "completed",
            json!({"message": "must not accompany success"}),
        ),
        (
            "interrupted",
            json!({"message": "must not accompany interruption"}),
        ),
    ] {
        assert_eq!(
            AppServerProtocol::parse_turn_completed(
                &json!({
                    "method": "turn/completed",
                    "params": {
                        "threadId": "thr_123",
                        "turn": {
                            "id": "turn_1",
                            "items": [],
                            "status": status,
                            "error": error
                        }
                    }
                }),
                "thr_123",
                "turn_1"
            ),
            Err(ProtocolError::MalformedTerminal)
        );
    }

    for invalid_error in [json!("not-an-object"), json!({}), json!({"message": 7})] {
        assert_eq!(
            AppServerProtocol::parse_turn_completed(
                &json!({
                    "method": "turn/completed",
                    "params": {
                        "threadId": "thr_123",
                        "turn": {
                            "id": "turn_1",
                            "items": [],
                            "status": "failed",
                            "error": invalid_error
                        }
                    }
                }),
                "thr_123",
                "turn_1"
            ),
            Err(ProtocolError::MalformedTerminal)
        );
    }

    let failed_without_error = AppServerProtocol::parse_turn_completed(
        &json!({
            "method": "turn/completed",
            "params": {
                "threadId": "thr_123",
                "turn": {
                    "id": "turn_1",
                    "items": [],
                    "status": "failed",
                    "error": null
                }
            }
        }),
        "thr_123",
        "turn_1",
    )
    .expect("the pinned schema permits a null failed error")
    .expect("failed is terminal evidence");
    assert_eq!(failed_without_error.status, TurnStatus::Failed);
    assert_eq!(failed_without_error.error_message, None);
}

#[test]
fn rejects_yielded_or_unfinished_tools_as_completed_delivery_evidence() {
    for items in [
        json!([{
            "id": "tool_exec",
            "type": "dynamicToolCall",
            "tool": "exec",
            "arguments": {},
            "status": "completed",
            "success": true,
            "contentItems": [{
                "type": "inputText",
                "text": "Script running with cell ID cell-7\nWall time: 11.0 seconds"
            }]
        }]),
        json!([{
            "id": "tool_exec",
            "type": "dynamicToolCall",
            "tool": "exec",
            "arguments": {},
            "status": "inProgress",
            "success": null,
            "contentItems": null
        }]),
        json!([
            {
                "id": "tool_exec",
                "type": "dynamicToolCall",
                "tool": "exec",
                "arguments": {},
                "status": "completed",
                "success": true,
                "contentItems": [{
                    "type": "inputText",
                    "text": "Script running with cell ID cell-7"
                }]
            },
            {
                "id": "tool_wait",
                "type": "dynamicToolCall",
                "tool": "wait",
                "arguments": {"cell_id": "cell-7", "terminate": true},
                "status": "completed",
                "success": true,
                "contentItems": [{
                    "type": "inputText",
                    "text": "Script completed\nExit code: 0"
                }]
            }
        ]),
    ] {
        assert_eq!(
            completed_session(
                items.as_array().expect("test items are an array").clone(),
                official_completed_terminal("notLoaded"),
            ),
            Err(SessionError::Terminal(
                ProtocolError::IncompleteToolExecution
            ))
        );
    }
}

#[test]
fn rejects_truncated_or_missing_separate_delivery_tool_evidence() {
    for completed_items in [vec![], vec![completed_exec("tool_apply")]] {
        assert_eq!(
            completed_session(completed_items, official_completed_terminal("notLoaded"),),
            Err(SessionError::Terminal(
                ProtocolError::IncompleteToolExecution
            ))
        );
    }
}

#[test]
fn requires_exact_two_execs_and_explicit_success_evidence() {
    let terminal = official_completed_terminal("notLoaded");
    let mut missing_success = completed_exec("tool_apply");
    missing_success
        .as_object_mut()
        .expect("test item is an object")
        .remove("success");
    let mut null_success = completed_exec("tool_apply");
    null_success["success"] = Value::Null;
    let mut false_success = completed_exec("tool_apply");
    false_success["success"] = json!(false);
    let mut policy_declined = completed_exec("tool_shell_write");
    policy_declined["status"] = json!("declined");
    policy_declined["success"] = json!(false);
    policy_declined["contentItems"][0]["text"] = json!("Tool execution declined by command policy");
    let mut arbitrary_tool = completed_exec("tool_apply");
    arbitrary_tool["tool"] = json!("mcp");
    let orphan_wait = json!({
        "id": "tool_wait",
        "type": "dynamicToolCall",
        "tool": "wait",
        "arguments": {"cell_id": "cell-7"},
        "status": "completed",
        "success": true,
        "contentItems": [{
            "type": "inputText",
            "text": "Script completed\nExit code: 0"
        }]
    });

    for completed_items in [
        vec![missing_success, completed_exec("tool_verify")],
        vec![null_success, completed_exec("tool_verify")],
        vec![false_success, completed_exec("tool_verify")],
        vec![policy_declined, completed_exec("tool_verify")],
        vec![arbitrary_tool, completed_exec("tool_verify")],
        vec![orphan_wait, completed_exec("tool_verify")],
        vec![
            completed_exec("tool_apply"),
            completed_exec("tool_verify"),
            completed_exec("tool_extra"),
        ],
    ] {
        assert_eq!(
            completed_session(completed_items, terminal.clone()),
            Err(SessionError::Terminal(
                ProtocolError::IncompleteToolExecution
            ))
        );
    }
}

#[test]
fn rejects_nested_shell_nonzero_even_when_outer_exec_reports_success() {
    let mut failed_write = completed_exec("tool_shell_write");
    failed_write["contentItems"][0]["text"] =
        json!("Script completed\nWall time: 0.2 seconds\nOutput:\nExit code: 7\nWrite failed");

    assert_eq!(
        completed_session(
            vec![failed_write, completed_exec("tool_shell_verify")],
            official_completed_terminal("notLoaded"),
        ),
        Err(SessionError::Terminal(
            ProtocolError::IncompleteToolExecution
        ))
    );
}

#[test]
fn rejects_missing_nested_shell_exit_evidence() {
    let mut ambiguous_write = completed_exec("tool_shell_write");
    ambiguous_write["contentItems"][0]["text"] =
        json!("Script completed\nWall time: 0.2 seconds\nOutput:\nambiguous result");

    assert_eq!(
        completed_session(
            vec![ambiguous_write, completed_exec("tool_shell_verify")],
            official_completed_terminal("notLoaded"),
        ),
        Err(SessionError::Terminal(
            ProtocolError::IncompleteToolExecution
        ))
    );
}

#[test]
fn accepts_only_two_completed_official_command_executions() {
    let outcome = completed_session(
        vec![
            completed_command_execution("command_shell_write", "completed"),
            completed_command_execution("command_shell_verify", "completed"),
        ],
        official_completed_terminal("notLoaded"),
    )
    .expect("the exact official commandExecution sequence is accepted")
    .expect("this is a terminal notification");
    assert_eq!(outcome.status, TurnStatus::Completed);

    let mut forged_success = completed_command_execution("command_shell_write", "completed");
    forged_success["success"] = json!(true);
    let mut nonzero_exit = completed_command_execution("command_shell_write", "completed");
    nonzero_exit["exitCode"] = json!(7);
    let mut missing_exit = completed_command_execution("command_shell_write", "completed");
    missing_exit
        .as_object_mut()
        .expect("command fixture is an object")
        .remove("exitCode");
    let mut null_exit = completed_command_execution("command_shell_write", "completed");
    null_exit["exitCode"] = Value::Null;
    let mut missing_command = completed_command_execution("command_shell_write", "completed");
    missing_command
        .as_object_mut()
        .expect("command fixture is an object")
        .remove("command");
    let mut missing_cwd = completed_command_execution("command_shell_write", "completed");
    missing_cwd
        .as_object_mut()
        .expect("command fixture is an object")
        .remove("cwd");
    let mut malformed_actions = completed_command_execution("command_shell_write", "completed");
    malformed_actions["commandActions"] = json!({});
    for completed_items in [
        vec![
            completed_command_execution("command_shell_write", "failed"),
            completed_command_execution("command_shell_verify", "completed"),
        ],
        vec![
            completed_command_execution("command_shell_write", "declined"),
            completed_command_execution("command_shell_verify", "completed"),
        ],
        vec![
            forged_success,
            completed_command_execution("command_shell_verify", "completed"),
        ],
        vec![
            nonzero_exit,
            completed_command_execution("command_shell_verify", "completed"),
        ],
        vec![
            missing_exit,
            completed_command_execution("command_shell_verify", "completed"),
        ],
        vec![
            null_exit,
            completed_command_execution("command_shell_verify", "completed"),
        ],
        vec![
            missing_command,
            completed_command_execution("command_shell_verify", "completed"),
        ],
        vec![
            missing_cwd,
            completed_command_execution("command_shell_verify", "completed"),
        ],
        vec![
            malformed_actions,
            completed_command_execution("command_shell_verify", "completed"),
        ],
        vec![
            completed_command_execution("command_shell_write", "completed"),
            completed_exec("tool_shell_verify"),
        ],
        vec![
            completed_command_execution("command_shell_write", "completed"),
            completed_command_execution("command_shell_verify", "completed"),
            completed_command_execution("command_extra", "completed"),
        ],
    ] {
        assert_eq!(
            completed_session(completed_items, official_completed_terminal("notLoaded")),
            Err(SessionError::Terminal(
                ProtocolError::IncompleteToolExecution
            ))
        );
    }
}

#[test]
fn command_execution_aggregated_output_must_be_present_null_or_string() {
    for output in [Value::Null, json!("written")] {
        let mut write = completed_command_execution("command_shell_write", "completed");
        write["aggregatedOutput"] = output;
        let outcome = completed_session(
            vec![
                write,
                completed_command_execution("command_shell_verify", "completed"),
            ],
            official_completed_terminal("notLoaded"),
        )
        .expect("null and string aggregatedOutput values are valid")
        .expect("this is a terminal notification");
        assert_eq!(outcome.status, TurnStatus::Completed);
    }

    let mut missing = completed_command_execution("command_shell_write", "completed");
    missing
        .as_object_mut()
        .expect("command fixture is an object")
        .remove("aggregatedOutput");
    let mut array = completed_command_execution("command_shell_write", "completed");
    array["aggregatedOutput"] = json!([]);
    let mut number = completed_command_execution("command_shell_write", "completed");
    number["aggregatedOutput"] = json!(0);
    let mut object = completed_command_execution("command_shell_write", "completed");
    object["aggregatedOutput"] = json!({});

    for write in [missing, array, number, object] {
        assert_eq!(
            completed_session(
                vec![
                    write,
                    completed_command_execution("command_shell_verify", "completed"),
                ],
                official_completed_terminal("notLoaded"),
            ),
            Err(SessionError::Terminal(
                ProtocolError::IncompleteToolExecution
            ))
        );
    }
}

#[test]
fn rejects_terminal_embedded_tools_as_completed_item_evidence() {
    let terminal = json!({
        "method": "turn/completed",
        "params": {
            "threadId": "thr_123",
            "turn": {
                "id": "turn_456",
                "items": [completed_exec("tool_apply"), completed_exec("tool_verify")],
                "itemsView": "full",
                "status": "completed",
                "error": null
            }
        }
    });

    assert_eq!(
        AppServerProtocol::parse_turn_completed(&terminal, "thr_123", "turn_456"),
        Err(ProtocolError::MalformedTerminal)
    );
}

#[test]
fn accepts_a_yielded_exec_only_after_wait_observes_completed_success() {
    let completed_items = json!([
        {
            "id": "tool_exec",
            "type": "dynamicToolCall",
            "tool": "exec",
            "arguments": {},
            "status": "completed",
            "success": true,
            "contentItems": [{
                "type": "inputText",
                "text": "Script running with cell ID cell-7\nWall time: 11.0 seconds"
            }]
        },
        {
            "id": "tool_wait",
            "type": "dynamicToolCall",
            "tool": "wait",
            "arguments": {
                "cell_id": "cell-7",
                "yield_time_ms": 10000,
                "max_tokens": 1000
            },
            "status": "completed",
            "success": true,
            "contentItems": [{
                "type": "inputText",
                "text": "Script completed\nWall time: 0.2 seconds\nProcess exited with code 0"
            }]
        },
        completed_exec("tool_verify")
    ]);
    let outcome = completed_session(
        completed_items
            .as_array()
            .expect("completed items are an array")
            .clone(),
        official_completed_terminal("notLoaded"),
    )
    .expect("a completed wait resolves the yielded execution")
    .expect("this is a terminal notification");

    assert_eq!(outcome.status, TurnStatus::Completed);
}

#[test]
fn rejects_wait_cell_drift_multiple_yields_and_malformed_termination() {
    let yielded_exec = |id: &str, output: &str| {
        json!({
            "id": id,
            "type": "dynamicToolCall",
            "tool": "exec",
            "arguments": {},
            "status": "completed",
            "success": true,
            "contentItems": [{"type": "inputText", "text": output}]
        })
    };
    let wait = |id: &str, cell_id: &str, terminate: Value, output: &str| {
        json!({
            "id": id,
            "type": "dynamicToolCall",
            "tool": "wait",
            "arguments": {"cell_id": cell_id, "terminate": terminate},
            "status": "completed",
            "success": true,
            "contentItems": [{"type": "inputText", "text": output}]
        })
    };
    let completed = "Script completed\nExit code: 0";

    for completed_items in [
        vec![
            yielded_exec("tool_apply", "Script running with cell ID cell-A"),
            wait(
                "tool_wait_drift",
                "cell-A",
                json!(false),
                "Script running with cell ID cell-B",
            ),
            wait("tool_wait_b", "cell-B", json!(false), completed),
            wait("tool_wait_a", "cell-A", json!(false), completed),
            completed_exec("tool_verify"),
        ],
        vec![
            yielded_exec(
                "tool_apply",
                "Script running with cell ID cell-A\nScript running with cell ID cell-B",
            ),
            wait("tool_wait_a", "cell-A", json!(false), completed),
            wait("tool_wait_b", "cell-B", json!(false), completed),
            completed_exec("tool_verify"),
        ],
        vec![
            yielded_exec("tool_apply", "Script running with cell ID cell-A"),
            wait("tool_wait", "cell-A", json!("no"), completed),
            completed_exec("tool_verify"),
        ],
        vec![
            yielded_exec("tool_apply", "Script running with cell ID cell-A"),
            wait("tool_wait", "cell-A", Value::Null, completed),
            completed_exec("tool_verify"),
        ],
        vec![
            yielded_exec("tool_apply", "Script running with cell ID cell-A"),
            wait(
                "tool_wait",
                "cell-A",
                json!(false),
                "Script running with cell ID cell-A\nScript completed\nExit code: 0",
            ),
            completed_exec("tool_verify"),
        ],
        vec![
            yielded_exec("tool_shell_write", "Script running with cell ID cell-A"),
            wait(
                "tool_wait",
                "cell-A",
                json!(false),
                "Script completed\nExit code: 0\nOutput:\nExit code: 7",
            ),
            completed_exec("tool_shell_verify"),
        ],
        vec![
            yielded_exec(
                "tool_shell_write",
                "Script running with cell ID cell-A\nExit code: 7",
            ),
            wait("tool_wait", "cell-A", json!(false), completed),
            completed_exec("tool_shell_verify"),
        ],
    ] {
        assert_eq!(
            completed_session(completed_items, official_completed_terminal("notLoaded"),),
            Err(SessionError::Terminal(
                ProtocolError::IncompleteToolExecution
            ))
        );
    }
}

#[test]
fn accepts_official_completed_item_stream_with_summary_terminal() {
    let mut session = AppServerSession::new();
    for request in [
        SessionRequest::Initialize,
        SessionRequest::ThreadStart,
        SessionRequest::TurnStart,
    ] {
        session
            .mark_request_sent(request)
            .expect("each lifecycle request is sent once");
    }
    session
        .ingest(json!({
            "id": 0,
            "result": {
                "userAgent": "codex_cli_rs/0.146.0",
                "platformFamily": "windows",
                "platformOs": "windows",
                "codexHome": r"C:\lattice\codex-home"
            }
        }))
        .expect("initialize response is valid");
    session
        .ingest(json!({"id": 1, "result": {"thread": {"id": "thr_123"}}}))
        .expect("thread response is valid");
    session
        .ingest(json!({"id": 2, "result": {"turn": {"id": "turn_456"}}}))
        .expect("turn response is valid");

    for (id, command) in [
        ("tool_apply", "apply_patch fixture"),
        ("tool_verify", "verify fixture"),
    ] {
        assert_eq!(
            session.ingest(json!({
                "method": "item/completed",
                "params": {
                    "threadId": "thr_123",
                    "turnId": "turn_456",
                    "item": {
                        "id": id,
                        "type": "dynamicToolCall",
                        "tool": "exec",
                        "arguments": {"command": command},
                        "status": "completed",
                        "success": true,
                        "contentItems": [{
                            "type": "inputText",
                            "text": "Script completed\nExit code: 0"
                        }]
                    },
                    "completedAtMs": 1
                }
            })),
            Ok(None)
        );
    }

    let outcome = session
        .ingest(json!({
            "method": "turn/completed",
            "params": {
                "threadId": "thr_123",
                "turn": {
                    "id": "turn_456",
                    "items": [{
                        "id": "agent-final",
                        "type": "agentMessage",
                        "text": "Delivery complete."
                    }],
                    "itemsView": "summary",
                    "status": "completed",
                    "error": null
                }
            }
        }))
        .expect("official summary terminal should be accepted")
        .expect("terminal should complete the session");

    assert_eq!(outcome.status, TurnStatus::Completed);
}
