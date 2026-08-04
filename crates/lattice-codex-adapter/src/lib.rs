//! Supervised Codex app-server adapter.

use std::path::Path;

use serde_json::{Value, json};

/// Stable terminal states emitted by `turn/completed`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TurnStatus {
    Completed,
    Failed,
    Interrupted,
}

/// Terminal evidence bound to one exact app-server turn.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TurnOutcome {
    pub turn_id: String,
    pub status: TurnStatus,
    pub error_message: Option<String>,
}

/// Fail-closed protocol parsing failures.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtocolError {
    UnexpectedTurn,
    MalformedTerminal,
}

/// Builds and validates the stable subset of the Codex app-server protocol.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppServerProtocol {
    client_name: String,
    client_version: String,
}

impl AppServerProtocol {
    /// Creates a protocol client identity used by initialization and metrics.
    #[must_use]
    pub fn new(client_name: impl Into<String>, client_version: impl Into<String>) -> Self {
        Self {
            client_name: client_name.into(),
            client_version: client_version.into(),
        }
    }

    /// Builds the required first request for one stdio connection.
    #[must_use]
    pub fn initialize_request(&self, id: u64) -> Value {
        json!({
            "method": "initialize",
            "id": id,
            "params": {
                "clientInfo": {
                    "name": self.client_name,
                    "title": "LATTICE DevOS",
                    "version": self.client_version
                }
            }
        })
    }

    /// Builds the required acknowledgement after initialization succeeds.
    #[must_use]
    pub fn initialized_notification(&self) -> Value {
        json!({"method": "initialized", "params": {}})
    }

    /// Starts one non-interactive workspace-write thread in the bounded root.
    #[must_use]
    pub fn thread_start_request(&self, id: u64, working_directory: &Path) -> Value {
        let cwd = working_directory.to_string_lossy();
        json!({
            "method": "thread/start",
            "id": id,
            "params": {
                "cwd": cwd,
                "approvalPolicy": "never",
                "sandbox": "workspaceWrite",
                "serviceName": self.client_name
            }
        })
    }

    /// Starts one turn with network disabled and only the task root writable.
    #[must_use]
    pub fn turn_start_request(
        &self,
        id: u64,
        thread_id: &str,
        working_directory: &Path,
        prompt: &str,
    ) -> Value {
        let cwd = working_directory.to_string_lossy();
        json!({
            "method": "turn/start",
            "id": id,
            "params": {
                "threadId": thread_id,
                "input": [{"type": "text", "text": prompt}],
                "cwd": cwd,
                "approvalPolicy": "never",
                "sandboxPolicy": {
                    "type": "workspaceWrite",
                    "writableRoots": [cwd],
                    "networkAccess": false
                }
            }
        })
    }

    /// Parses only `turn/completed` and binds it to the expected turn.
    ///
    /// Non-terminal notifications return `Ok(None)`. A malformed terminal or
    /// terminal for another turn fails closed.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::MalformedTerminal`] when terminal evidence is
    /// incomplete and [`ProtocolError::UnexpectedTurn`] when it is not bound
    /// to the requested turn.
    pub fn parse_turn_completed(
        message: &Value,
        expected_turn_id: &str,
    ) -> Result<Option<TurnOutcome>, ProtocolError> {
        if message.get("method").and_then(Value::as_str) != Some("turn/completed") {
            return Ok(None);
        }

        let Some(turn) = message
            .get("params")
            .and_then(|params| params.get("turn"))
            .and_then(Value::as_object)
        else {
            return Err(ProtocolError::MalformedTerminal);
        };
        let Some(turn_id) = turn.get("id").and_then(Value::as_str) else {
            return Err(ProtocolError::MalformedTerminal);
        };
        if turn_id != expected_turn_id {
            return Err(ProtocolError::UnexpectedTurn);
        }
        let status = match turn.get("status").and_then(Value::as_str) {
            Some("completed") => TurnStatus::Completed,
            Some("failed") => TurnStatus::Failed,
            Some("interrupted") => TurnStatus::Interrupted,
            _ => return Err(ProtocolError::MalformedTerminal),
        };
        let error_message = turn
            .get("error")
            .filter(|error| !error.is_null())
            .and_then(|error| error.get("message"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);

        Ok(Some(TurnOutcome {
            turn_id: turn_id.to_owned(),
            status,
            error_message,
        }))
    }
}
