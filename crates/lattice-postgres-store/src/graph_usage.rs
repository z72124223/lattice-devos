//! Append-only per-call graph usage observations, separate from analysis receipts.
use crate::postgres_setup::verify_graph_usage_extension;
use postgres::{Client, IsolationLevel};
use serde_json::Value;

const ARGUMENTS: &str = "GRAPH_USAGE_ARGUMENTS_REJECTED";
const MAX_PAYLOAD_BYTES: usize = 4096;
const START_KEYS: &[&str] = &[
    "usage_id",
    "project_id",
    "task_ref",
    "commit",
    "operation",
    "integration_mode",
    "query_digest",
];
const FINISH_KEYS: &[&str] = &[
    "usage_id",
    "outcome",
    "source_receipt_digest",
    "record_count",
    "result_bytes",
    "duration_ms",
    "error_code",
    "analysis_calls",
    "query_calls",
];

/// Fixed Runtime-role API. No caller receives SQL or direct audit-table access.
pub struct PostgresGraphUsage {
    client: Client,
}

impl PostgresGraphUsage {
    /// Verifies the exact extension catalog and Runtime connection identity.
    ///
    /// # Errors
    /// Rejects absent upgrades, catalog drift or unavailable persistence.
    pub fn new(mut client: Client) -> Result<Self, &'static str> {
        let role: bool = client.query_one(
            "SELECT session_user='lattice_runtime_login' AND current_setting('role')='lattice_runtime'", &[])
            .map_err(usage_error)?.get(0);
        if !role {
            return Err("GRAPH_USAGE_CONTEXT_REJECTED");
        }
        if !verify_graph_usage_extension(&mut client).map_err(|_| "GRAPH_USAGE_STORE_REJECTED")? {
            return Err("GRAPH_USAGE_UPGRADE_REQUIRED");
        }
        Ok(Self { client })
    }

    /// Appends one immutable start; exact repeated payloads are idempotent.
    /// Task binding is validated by `PostgreSQL` against formal general submissions.
    ///
    /// # Errors
    /// Rejects invalid arguments, forged task binding, changed replay or unavailable storage.
    pub fn begin(&mut self, value: &Value) -> Result<(), &'static str> {
        validate_start(value)?;
        self.write("SELECT control_product.graph_usage_begin_v1($1)", value)
    }

    /// Appends the observed terminal result. Bytes refer only to inner result JSON.
    ///
    /// # Errors
    /// Rejects absent starts, invalid results, changed replay or unavailable storage.
    pub fn finish(&mut self, value: &Value) -> Result<(), &'static str> {
        validate_finish(value)?;
        self.write("SELECT control_product.graph_usage_finish_v1($1)", value)
    }

    /// Reads aggregate counts and at most twenty recent calls; never claims full coverage.
    ///
    /// # Errors
    /// Rejects malformed scopes, forged task binding or unavailable persistence.
    pub fn summary(
        &mut self,
        project_id: &str,
        task_ref: Option<&str>,
    ) -> Result<Value, &'static str> {
        if !valid_project(project_id) || task_ref.is_some_and(|v| !hex(v, &[64])) {
            return Err(ARGUMENTS);
        }
        let mut tx = self
            .client
            .build_transaction()
            .isolation_level(IsolationLevel::RepeatableRead)
            .read_only(true)
            .start()
            .map_err(usage_error)?;
        tx.batch_execute("SET LOCAL lock_timeout='5s'; SET LOCAL statement_timeout='30s'")
            .map_err(usage_error)?;
        let result: Value = tx
            .query_one(
                "SELECT control_product.graph_usage_summary_v1($1,$2)",
                &[&project_id, &task_ref],
            )
            .map_err(usage_error)?
            .try_get(0)
            .map_err(usage_error)?;
        tx.commit().map_err(usage_error)?;
        if result.to_string().len() > 131_072 {
            return Err("GRAPH_USAGE_OUTPUT_LIMIT_EXCEEDED");
        }
        Ok(result)
    }

    fn write(&mut self, sql: &str, value: &Value) -> Result<(), &'static str> {
        let mut tx = self.client.transaction().map_err(usage_error)?;
        tx.batch_execute("SET LOCAL lock_timeout='5s'; SET LOCAL statement_timeout='30s'")
            .map_err(usage_error)?;
        let disposition: String = tx
            .query_one(sql, &[value])
            .map_err(usage_error)?
            .try_get(0)
            .map_err(usage_error)?;
        if !matches!(disposition.as_str(), "RECORDED" | "REPLAYED") {
            return Err("GRAPH_USAGE_RESPONSE_REJECTED");
        }
        tx.commit().map_err(usage_error)
    }
}

fn closed(value: &Value, keys: &[&str]) -> bool {
    value.to_string().len() <= MAX_PAYLOAD_BYTES
        && value
            .as_object()
            .is_some_and(|o| o.len() == keys.len() && o.keys().all(|k| keys.contains(&k.as_str())))
}
fn hex(value: &str, lengths: &[usize]) -> bool {
    lengths.contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn optional_hex(value: &Value, lengths: &[usize]) -> bool {
    value.is_null() || value.as_str().is_some_and(|v| hex(v, lengths))
}
fn integer(value: &Value, max: i64) -> bool {
    value.as_i64().is_some_and(|v| (0..=max).contains(&v))
}
fn optional_integer(value: &Value, max: i64) -> bool {
    value.is_null() || integer(value, max)
}
fn valid_project(value: &str) -> bool {
    (2..=64).contains(&value.len())
        && (value.as_bytes()[0].is_ascii_lowercase() || value.as_bytes()[0].is_ascii_digit())
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
}
fn validate_start(value: &Value) -> Result<(), &'static str> {
    if !closed(value, START_KEYS)
        || !value["usage_id"].as_str().is_some_and(|v| hex(v, &[64]))
        || !value["project_id"].as_str().is_some_and(valid_project)
        || !optional_hex(&value["task_ref"], &[64])
        || !optional_hex(&value["commit"], &[40, 64])
        || !matches!(value["operation"].as_str(), Some("QUERY" | "REFRESH"))
        || !matches!(
            value["integration_mode"].as_str(),
            Some("CORE_ONLY" | "GRAPHIFY")
        )
        || !optional_hex(&value["query_digest"], &[64])
        || (value["operation"] == "QUERY"
            && (value["commit"].is_null() || value["query_digest"].is_null()))
    {
        return Err(ARGUMENTS);
    }
    Ok(())
}
fn validate_finish(value: &Value) -> Result<(), &'static str> {
    let error = &value["error_code"];
    let valid_error = error.is_null()
        || error.as_str().is_some_and(|v| {
            !v.is_empty()
                && v.len() <= 96
                && v.as_bytes()[0].is_ascii_uppercase()
                && v.bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
        });
    if !closed(value, FINISH_KEYS)
        || !value["usage_id"].as_str().is_some_and(|v| hex(v, &[64]))
        || !matches!(
            value["outcome"].as_str(),
            Some("QUERIED" | "ANALYZED" | "REUSED" | "FAILED")
        )
        || !optional_hex(&value["source_receipt_digest"], &[64])
        || !optional_integer(&value["record_count"], 100_000)
        || !optional_integer(&value["result_bytes"], 16_777_216)
        || !integer(&value["duration_ms"], 2_592_000_000)
        || !integer(&value["analysis_calls"], 1_000_000)
        || !integer(&value["query_calls"], 1)
        || !valid_error
        || (value["outcome"] == "FAILED" && error.is_null())
        || (value["outcome"] != "FAILED"
            && (!error.is_null()
                || value["source_receipt_digest"].is_null()
                || value["record_count"].is_null()
                || value["result_bytes"].is_null()))
        || (value["outcome"] == "ANALYZED" && value["analysis_calls"] == 0)
        || (matches!(value["outcome"].as_str(), Some("REUSED" | "QUERIED"))
            && value["analysis_calls"] != 0)
        || (value["outcome"] == "QUERIED" && value["query_calls"] != 1)
    {
        return Err(ARGUMENTS);
    }
    Ok(())
}

#[allow(clippy::needless_pass_by_value)]
fn usage_error(error: postgres::Error) -> &'static str {
    let Some(database) = error.as_db_error() else {
        return "GRAPH_USAGE_DATABASE_UNAVAILABLE";
    };
    if matches!(database.code().code(), "42883" | "42703" | "42P01") {
        return "GRAPH_USAGE_UPGRADE_REQUIRED";
    }
    match database.message() {
        "GRAPH_USAGE_ARGUMENTS_REJECTED" => ARGUMENTS,
        "GRAPH_USAGE_CONTEXT_REJECTED" => "GRAPH_USAGE_CONTEXT_REJECTED",
        "GRAPH_USAGE_TASK_BINDING_REJECTED" => "GRAPH_USAGE_TASK_BINDING_REJECTED",
        "GRAPH_USAGE_IDEMPOTENCY_CONFLICT" => "GRAPH_USAGE_IDEMPOTENCY_CONFLICT",
        "GRAPH_USAGE_START_REQUIRED" => "GRAPH_USAGE_START_REQUIRED",
        "GRAPH_USAGE_OUTCOME_REJECTED" => "GRAPH_USAGE_OUTCOME_REJECTED",
        _ => "GRAPH_USAGE_DATABASE_UNAVAILABLE",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn start() -> Value {
        json!({"usage_id":"1".repeat(64),"project_id":"sample","task_ref":null,"commit":"a".repeat(40),"operation":"QUERY","integration_mode":"CORE_ONLY","query_digest":"2".repeat(64)})
    }
    fn finish() -> Value {
        json!({"usage_id":"1".repeat(64),"outcome":"QUERIED","source_receipt_digest":"3".repeat(64),"record_count":0,"result_bytes":123,"duration_ms":1,"error_code":null,"analysis_calls":0,"query_calls":1})
    }
    #[test]
    fn usage_payloads_reject_unknown_private_fields_and_bad_identities() {
        assert!(validate_start(&start()).is_ok());
        for (key, value) in [
            ("query", json!("private code")),
            ("project_id", json!("../other")),
            ("task_ref", json!("bad")),
            ("commit", json!(null)),
            ("integration_mode", json!("FULL_CHAIN")),
            ("query_digest", json!(1)),
        ] {
            let mut input = start();
            input[key] = value;
            assert_eq!(validate_start(&input), Err(ARGUMENTS), "{key}");
        }
        let mut unbound = start();
        unbound["operation"] = json!("REFRESH");
        unbound["commit"] = Value::Null;
        unbound["query_digest"] = Value::Null;
        assert!(validate_start(&unbound).is_ok());
    }
    #[test]
    fn usage_results_require_observed_counters_and_bounded_metadata() {
        assert!(validate_finish(&finish()).is_ok());
        for (key, value) in [
            ("query_calls", json!(0)),
            ("analysis_calls", json!(1)),
            ("result_bytes", json!(16_777_217)),
            ("record_count", json!(-1)),
            ("duration_ms", json!(1.5)),
            ("error_code", json!("SECRET=private")),
            ("source_receipt_digest", Value::Null),
            ("unknown", json!("secret")),
        ] {
            let mut input = finish();
            input[key] = value;
            assert_eq!(validate_finish(&input), Err(ARGUMENTS), "{key}");
        }
        let mut failed = finish();
        failed["outcome"] = json!("FAILED");
        failed["error_code"] = json!("QUERY_FAILED");
        failed["source_receipt_digest"] = Value::Null;
        failed["query_calls"] = json!(0);
        assert!(validate_finish(&failed).is_ok());
        failed["error_code"] = Value::Null;
        assert_eq!(validate_finish(&failed), Err(ARGUMENTS));
    }
    #[test]
    fn usage_summary_scope_identifiers_are_closed() {
        for value in ["", "a", "../other", "漢字", "Project", "x\n"] {
            assert!(!valid_project(value));
        }
        assert!(valid_project("project_01"));
        assert!(valid_project("01"));
    }
}
