use std::path::Path;

use lattice_codex_adapter::{AppServerProtocol, ProtocolError, TurnOutcome, TurnStatus};
use serde_json::json;

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
    let outcome = AppServerProtocol::parse_turn_completed(
        &json!({
            "method": "turn/completed",
            "params": {
                "threadId": "thr_123",
                "turn": {"id": "turn_456", "status": "completed", "items": [], "error": null}
            }
        }),
        "thr_123",
        "turn_456",
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
            AppServerProtocol::parse_turn_completed(
                &json!({
                    "method": "turn/completed",
                    "params": {
                        "threadId": "thr_123",
                        "turn": {
                            "id": "turn_1",
                            "items": items,
                            "itemsView": "full",
                            "status": "completed",
                            "error": null
                        }
                    }
                }),
                "thr_123",
                "turn_1"
            ),
            Err(ProtocolError::IncompleteToolExecution)
        );
    }
}

#[test]
fn accepts_a_yielded_exec_only_after_wait_observes_completed_success() {
    let outcome = AppServerProtocol::parse_turn_completed(
        &json!({
            "method": "turn/completed",
            "params": {
                "threadId": "thr_123",
                "turn": {
                    "id": "turn_1",
                    "items": [
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
                                "text": "Script completed\nWall time: 0.2 seconds\nExit code: 0"
                            }]
                        }
                    ],
                    "itemsView": "full",
                    "status": "completed",
                    "error": null
                }
            }
        }),
        "thr_123",
        "turn_1",
    )
    .expect("a completed wait resolves the yielded execution")
    .expect("this is a terminal notification");

    assert_eq!(outcome.status, TurnStatus::Completed);
}
