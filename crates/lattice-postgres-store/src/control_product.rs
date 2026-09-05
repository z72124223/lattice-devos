//! Product observations share the verified Store and the existing Task Ledger.
use crate::MigrationTarget;
use crate::postgres_setup::verify_runtime_store_schema;
use lattice_contracts::ContentDigest;
use postgres::{Client, IsolationLevel};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// Closed product commands. None can set a task's formal completion state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ControlProductCommand {
    /// Acceptance criteria, display priority and task relationships.
    Metadata {
        task_ref: ContentDigest,
        request_id: String,
        expected_revision: i64,
        title: String,
        success_criteria: String,
        priority: i32,
        parent_ref: Option<String>,
        dependency_refs: Vec<String>,
    },
    /// The one saved Codex conversation for one execution or verification phase.
    Claim {
        task_ref: ContentDigest,
        claim_id: String,
        phase: String,
        prompt: String,
        model: String,
        worktree_path: String,
    },
    /// A bounded observation of an actual Codex operation.
    Observe {
        task_ref: ContentDigest,
        claim_id: String,
        request_id: String,
        expected_sequence: i64,
        kind: String,
        thread_id: Option<String>,
        turn_id: Option<String>,
        summary: String,
        evidence_ref: Option<String>,
        approval_id: Option<String>,
        decision: Option<String>,
        input_id: Option<String>,
        payload: Option<Value>,
    },
    /// An explicit project decision, with an optional replacement relationship.
    Decision {
        decision_id: String,
        project_id: String,
        task_ref: Option<String>,
        subject: String,
        content: String,
        reason: String,
        source: String,
        source_reference: String,
        supersedes_id: Option<String>,
        client_request_id: String,
        expected_revision: i64,
        expected_digest: String,
    },
}

impl ControlProductCommand {
    fn digest(&self) -> String {
        let value = match self {
            Self::Metadata {
                task_ref,
                request_id,
                title,
                success_criteria,
                priority,
                parent_ref,
                dependency_refs,
                ..
            } => json!({
                "action":"METADATA","task_ref":task_ref.as_str(),"request_id":request_id,
                "title":title,"success_criteria":success_criteria,"priority":priority,
                "parent_ref":parent_ref,"dependency_refs":dependency_refs,
            }),
            Self::Claim {
                task_ref,
                claim_id,
                phase,
                prompt,
                model,
                worktree_path,
            } => json!({
                "action":"CLAIM","task_ref":task_ref.as_str(),"claim_id":claim_id,
                "phase":phase,"prompt":prompt,"model":model,"worktree_path":worktree_path,
            }),
            Self::Observe {
                task_ref,
                claim_id,
                request_id,
                kind,
                thread_id,
                turn_id,
                summary,
                evidence_ref,
                approval_id,
                decision,
                input_id,
                payload,
                ..
            } => json!({
                "action":"OBSERVE","task_ref":task_ref.as_str(),"claim_id":claim_id,"request_id":request_id,"kind":kind,
                "thread_id":thread_id,"turn_id":turn_id,"summary":summary,
                "evidence_ref":evidence_ref,"approval_id":approval_id,"decision":decision,
                "input_id":input_id,"payload":payload,
            }),
            Self::Decision {
                decision_id,
                project_id,
                task_ref,
                subject,
                content,
                reason,
                source,
                source_reference,
                supersedes_id,
                client_request_id,
                expected_revision,
                expected_digest,
            } => json!({
                "action":"DECISION","decision_id":decision_id,"project_id":project_id,
                "task_ref":task_ref,"subject":subject,"content":content,"reason":reason,
                "source":source,"source_reference":source_reference,"supersedes_id":supersedes_id,
                "client_request_id":client_request_id,"expected_revision":expected_revision,"expected_digest":expected_digest,
            }),
        };
        let mut hash = Sha256::new();
        hash.update(b"LATTICE_CONTROL_PRODUCT_COMMAND_V1\0");
        hash.update(value.to_string().as_bytes());
        let mut encoded = String::with_capacity(64);
        for byte in hash.finalize() {
            const HEX: &[u8; 16] = b"0123456789abcdef";
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 15)]));
        }
        encoded
    }
}

/// Fixed Runtime-role adapter. Construction verifies the complete Store profile.
pub struct PostgresControlProduct {
    client: Client,
}

impl PostgresControlProduct {
    /// Verifies the connection before allowing product reads or writes.
    ///
    /// # Errors
    /// Returns a stable code when the configured Store is unavailable or incompatible.
    pub fn new(mut client: Client, target: &MigrationTarget) -> Result<Self, &'static str> {
        verify_runtime_store_schema(&mut client, target)
            .map_err(|_| "CONTROL_PRODUCT_STORE_REJECTED")?;
        let present: bool = client
            .query_one("SELECT to_regnamespace('control_product') IS NOT NULL", &[])
            .map_err(product_error)?
            .get(0);
        if !present {
            return Err("CONTROL_PRODUCT_UPGRADE_REQUIRED");
        }
        Ok(Self { client })
    }

    /// Reads a bounded page of task references; callers verify each retained task.
    ///
    /// # Errors
    /// Rejects invalid bounds or an unavailable Store.
    pub fn task_refs(
        &mut self,
        project: &str,
        after: &str,
        limit: i32,
    ) -> Result<Vec<String>, &'static str> {
        if !(1..=32).contains(&limit) {
            return Err("CONTROL_PRODUCT_INPUT_REJECTED");
        }
        self.client
            .query(
                "SELECT * FROM control_product.task_refs_v1($1,$2,$3)",
                &[&project, &after, &limit],
            )
            .map_err(product_error)?
            .iter()
            .map(|row| {
                row.try_get(0)
                    .map_err(|_| "CONTROL_PRODUCT_RESPONSE_REJECTED")
            })
            .collect()
    }

    /// Reads metadata and restart state from the same bounded product snapshot.
    ///
    /// # Errors
    /// Rejects invalid bounds or an unavailable Store.
    pub fn snapshot(&mut self, project: &str, task_refs: &[String]) -> Result<Value, &'static str> {
        if task_refs.len() > 32 {
            return Err("CONTROL_PRODUCT_INPUT_REJECTED");
        }
        self.client
            .query_one(
                "SELECT control_product.snapshot_v1($1,$2)",
                &[&project, &task_refs],
            )
            .map_err(product_error)?
            .try_get(0)
            .map_err(|_| "CONTROL_PRODUCT_RESPONSE_REJECTED")
    }

    /// Reads full, bounded decision content from a single `PostgreSQL` snapshot.
    ///
    /// # Errors
    /// Reports invalid selectors, stale heads and unavailable decision storage.
    #[allow(clippy::too_many_arguments)]
    pub fn decisions(
        &mut self,
        mode: &str,
        scope: Option<&str>,
        decision_id: Option<&str>,
        subject: Option<&str>,
        query: Option<&str>,
        limit: Option<i32>,
        depth: Option<i32>,
        revision: Option<i64>,
        digest: Option<&str>,
    ) -> Result<Value, &'static str> {
        let mut transaction = self
            .client
            .build_transaction()
            .isolation_level(IsolationLevel::Serializable)
            .read_only(true)
            .start()
            .map_err(product_error)?;
        let value = transaction
            .query_one(
                "SELECT control_product.decision_snapshot_v1($1,$2,$3,$4,$5,$6,$7,$8,$9)",
                &[
                    &mode,
                    &scope,
                    &decision_id,
                    &subject,
                    &query,
                    &limit,
                    &depth,
                    &revision,
                    &digest,
                ],
            )
            .map_err(product_error)?
            .try_get(0)
            .map_err(|_| "CONTROL_PRODUCT_RESPONSE_REJECTED")?;
        transaction.commit().map_err(product_error)?;
        Ok(value)
    }

    /// Retrieves one saved approval or answer without a history-preview dependency.
    ///
    /// # Errors
    /// Reports an unavailable Store or an invalid response shape.
    pub fn question_resolution(
        &mut self,
        project: &str,
        task_ref: &str,
        question: &str,
    ) -> Result<Value, &'static str> {
        self.client
            .query_one(
                "SELECT control_product.question_resolution_v1($1,$2,$3)",
                &[&project, &task_ref, &question],
            )
            .map_err(product_error)?
            .try_get::<_, Option<Value>>(0)
            .map(|value| value.unwrap_or(Value::Null))
            .map_err(|_| "CONTROL_PRODUCT_RESPONSE_REJECTED")
    }

    /// Saves one closed command atomically; a retry reads its original result.
    ///
    /// # Errors
    /// Reports scope, lifecycle, idempotency, revision and database failures.
    #[allow(clippy::too_many_lines)]
    pub fn execute(&mut self, command: &ControlProductCommand) -> Result<Value, &'static str> {
        let digest = command.digest();
        let mut transaction = self
            .client
            .build_transaction()
            .isolation_level(IsolationLevel::Serializable)
            .start()
            .map_err(product_error)?;
        transaction
            .batch_execute("SET LOCAL lock_timeout='5s'; SET LOCAL statement_timeout='30s'")
            .map_err(product_error)?;
        let row = match command {
            ControlProductCommand::Metadata { task_ref, request_id, expected_revision,
                title, success_criteria, priority, parent_ref, dependency_refs } => {
                transaction.query_one(
                    "SELECT control_product.metadata_write_v1($1,$2,$3,$4,$5,$6,$7,$8,$9)",
                    &[&task_ref.as_str(),request_id,&digest,expected_revision,title,success_criteria,
                        priority,parent_ref,dependency_refs])
            }
            ControlProductCommand::Claim { task_ref, claim_id, phase, prompt, model, worktree_path } => {
                transaction.query_one("SELECT control_product.claim_v1($1,$2,$3,$4,$5,$6,$7)",
                    &[&task_ref.as_str(),claim_id,&digest,phase,prompt,model,worktree_path])
            }
            ControlProductCommand::Observe { claim_id, request_id, expected_sequence, kind,
                thread_id, turn_id, summary, evidence_ref, approval_id, decision, input_id, payload, .. } => {
                transaction.query_one(
                    "SELECT control_product.observe_v1($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)",
                    &[claim_id,request_id,&digest,expected_sequence,kind,thread_id,turn_id,summary,
                        evidence_ref,approval_id,decision,input_id,payload])
            }
            ControlProductCommand::Decision { decision_id, project_id, task_ref, subject,
                content, reason, source, source_reference, supersedes_id, client_request_id, expected_revision, expected_digest } => {
                transaction.query_one("SELECT control_product.decision_write_v1($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13)",
                    &[decision_id,project_id,task_ref,subject,content,reason,source,source_reference,supersedes_id,
                        client_request_id,expected_revision,expected_digest,&digest])
            }
        }.map_err(product_error)?;
        let value = row
            .try_get::<_, Value>(0)
            .map_err(|_| "CONTROL_PRODUCT_RESPONSE_REJECTED")?;
        transaction
            .commit()
            .map_err(|_| "CONTROL_PRODUCT_OUTCOME_UNKNOWN")?;
        Ok(value)
    }
}

// Consumes the driver error as a map_err callback; only a stable code escapes.
#[allow(clippy::needless_pass_by_value)]
fn product_error(error: postgres::Error) -> &'static str {
    let Some(database) = error.as_db_error() else {
        return "CONTROL_PRODUCT_DATABASE_UNAVAILABLE";
    };
    if matches!(database.code().code(), "40001" | "40P01") {
        return "CONTROL_PRODUCT_REVISION_CONFLICT";
    }
    if matches!(database.code().code(), "42883" | "42703" | "42P01") {
        return "CONTROL_PRODUCT_UPGRADE_REQUIRED";
    }
    match database.message() {
        "CONTROL_PRODUCT_TASK_MISSING" => "CONTROL_PRODUCT_TASK_MISSING",
        "DECISION_REVISION_MISMATCH" => "DECISION_REVISION_MISMATCH",
        "DECISION_NOT_FOUND" => "DECISION_NOT_FOUND",
        "DECISION_IDEMPOTENCY_CONFLICT" => "DECISION_IDEMPOTENCY_CONFLICT",
        "DECISION_CURRENT_EXISTS" => "DECISION_CURRENT_EXISTS",
        "DECISION_SUPERSESSION_TARGET_NOT_FOUND" => "DECISION_SUPERSESSION_TARGET_NOT_FOUND",
        "DECISION_CROSS_SCOPE_SUPERSESSION_REJECTED" => {
            "DECISION_CROSS_SCOPE_SUPERSESSION_REJECTED"
        }
        "DECISION_SUPERSESSION_TARGET_NOT_CURRENT" => "DECISION_SUPERSESSION_TARGET_NOT_CURRENT",
        "DECISION_STORE_LIMIT_EXCEEDED" => "DECISION_STORE_LIMIT_EXCEEDED",
        "DECISION_SOURCE_REJECTED" => "DECISION_SOURCE_REJECTED",
        "DECISION_OUTPUT_LIMIT_EXCEEDED" => "DECISION_OUTPUT_LIMIT_EXCEEDED",
        "CONTROL_PRODUCT_IDEMPOTENCY_CONFLICT" => "CONTROL_PRODUCT_IDEMPOTENCY_CONFLICT",
        "CONTROL_PRODUCT_REVISION_CONFLICT" => "CONTROL_PRODUCT_REVISION_CONFLICT",
        "CONTROL_PRODUCT_ACCEPTANCE_ALREADY_STARTED" => {
            "CONTROL_PRODUCT_ACCEPTANCE_ALREADY_STARTED"
        }
        "CONTROL_PRODUCT_RELATION_REJECTED" => "CONTROL_PRODUCT_RELATION_REJECTED",
        "CONTROL_PRODUCT_RELATION_CYCLE" => "CONTROL_PRODUCT_RELATION_CYCLE",
        "CONTROL_PRODUCT_EXECUTION_ALREADY_CLAIMED" => "CONTROL_PRODUCT_EXECUTION_ALREADY_CLAIMED",
        "CONTROL_PRODUCT_EXECUTION_REJECTED" => "CONTROL_PRODUCT_EXECUTION_REJECTED",
        "CONTROL_PRODUCT_EXECUTION_NOT_FINISHED" => "CONTROL_PRODUCT_EXECUTION_NOT_FINISHED",
        "CONTROL_PRODUCT_VERIFICATION_STALE" => "CONTROL_PRODUCT_VERIFICATION_STALE",
        "CONTROL_PRODUCT_TEXT_LIMIT_EXCEEDED" => "CONTROL_PRODUCT_TEXT_LIMIT_EXCEEDED",
        "CONTROL_PRODUCT_PROJECT_ALREADY_EXECUTING" => "CONTROL_PRODUCT_PROJECT_ALREADY_EXECUTING",
        "CONTROL_PRODUCT_CLAIM_MISSING" => "CONTROL_PRODUCT_CLAIM_MISSING",
        "CONTROL_PRODUCT_CLAIM_FAILURE_REJECTED" => "CONTROL_PRODUCT_CLAIM_FAILURE_REJECTED",
        "CONTROL_PRODUCT_THREAD_MISMATCH" => "CONTROL_PRODUCT_THREAD_MISMATCH",
        "CONTROL_PRODUCT_TURN_MISMATCH" => "CONTROL_PRODUCT_TURN_MISMATCH",
        "CONTROL_PRODUCT_INPUT_REJECTED" => "CONTROL_PRODUCT_INPUT_REJECTED",
        "CONTROL_PRODUCT_DISPATCH_REJECTED" => "CONTROL_PRODUCT_DISPATCH_REJECTED",
        "CONTROL_PRODUCT_DEPENDENCY_NOT_COMPLETED" => "CONTROL_PRODUCT_DEPENDENCY_NOT_COMPLETED",
        "CONTROL_PRODUCT_ARCHIVE_REJECTED" => "CONTROL_PRODUCT_ARCHIVE_REJECTED",
        "CONTROL_PRODUCT_VERIFICATION_REJECTED" => "CONTROL_PRODUCT_VERIFICATION_REJECTED",
        "CONTROL_PRODUCT_QUESTION_REJECTED" => "CONTROL_PRODUCT_QUESTION_REJECTED",
        "CONTROL_PRODUCT_UNEXPECTED_QUESTION" => "CONTROL_PRODUCT_UNEXPECTED_QUESTION",
        "CONTROL_PRODUCT_DECISION_SCOPE_REJECTED" => "CONTROL_PRODUCT_DECISION_SCOPE_REJECTED",
        _ => "CONTROL_PRODUCT_DATABASE_REJECTED",
    }
}
