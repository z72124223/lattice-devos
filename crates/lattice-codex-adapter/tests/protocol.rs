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
        json!({"method": "initialized", "params": {}})
    );
    assert_eq!(
        protocol.thread_start_request(1, Path::new(r"C:\work\fixture")),
        json!({
            "method": "thread/start",
            "id": 1,
            "params": {
                "cwd": r"C:\work\fixture",
                "approvalPolicy": "never",
                "sandbox": "workspaceWrite",
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
                "turn": {"id": "turn_456", "status": "completed", "items": [], "error": null}
            }
        }),
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
                "params": {"turn": {"id": "other", "status": "completed"}}
            }),
            "turn_456"
        ),
        Err(ProtocolError::UnexpectedTurn)
    );
}

#[test]
fn preserves_failed_and_interrupted_terminal_outcomes() {
    let failed = AppServerProtocol::parse_turn_completed(
        &json!({
            "method": "turn/completed",
            "params": {
                "turn": {
                    "id": "turn_1",
                    "status": "failed",
                    "error": {"message": "model unavailable"}
                }
            }
        }),
        "turn_1",
    )
    .expect("a typed failed terminal remains observable")
    .expect("this is a terminal notification");
    assert_eq!(failed.status, TurnStatus::Failed);
    assert_eq!(failed.error_message.as_deref(), Some("model unavailable"));

    let interrupted = AppServerProtocol::parse_turn_completed(
        &json!({
            "method": "turn/completed",
            "params": {"turn": {"id": "turn_2", "status": "interrupted"}}
        }),
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
            "turn_1"
        )
        .expect("unrelated valid notifications are ignored"),
        None
    );

    assert_eq!(
        AppServerProtocol::parse_turn_completed(
            &json!({"method": "turn/completed", "params": {"turn": {"id": "turn_1"}}}),
            "turn_1"
        ),
        Err(ProtocolError::MalformedTerminal)
    );
}
