//! Live durable Task Ledger repository backed by fixed `PostgreSQL` functions.

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt;

use lattice_cjson::{CanonicalValue, HashDomain, canonical_sha256};
use lattice_contracts::{
    CONTRACT_VERSION, ContentDigest, DaemonEpoch, ProjectId, ProjectSnapshotId, ResourceCounters,
    RuntimeAdmissionMode, RuntimeKind, STORE_CONTRACT_VERSION, STORE_PRODUCER_ID,
    STORE_PRODUCER_VERSION, StoreAuthorityHead, StoreAuthorityRevision, StoreDaemonInstanceId,
    StoreDurability, StoreMutationCommitment, StorePersistenceEvidence, StorePhysicalHead,
    StoreReceiptDisposition, StoreRepositoryOwner, StoreRevision, StoreScope, StoreTransactionId,
    StoreTransactionReceipt, StoreTransactionRequest, TASK_LEDGER_PRODUCER_ID,
    TASK_LEDGER_PRODUCER_VERSION, TaskId, TaskLedgerStreamHead, TaskLedgerStreamIdentity,
    TaskLedgerSubjectKind, WriterLeaseAuthorityHead, WriterLeaseStatus,
    valid_task_ingress_client_request_id,
};
use lattice_foreman_state::{
    Confidence, EpistemicReferences, ForemanSnapshot, ForemanState, RefreshTrigger,
};
use lattice_task_ledger::{
    AppendCommand, AutonomyReceiptAppendPlan, CommandId, CommandReceipt, Diagnostic,
    FOREMAN_RECORD_SCHEMA, ForemanSnapshotAppendPlan, LEDGER_CHECKPOINT_SCHEMA_VERSION,
    LEDGER_SCHEMA_VERSION, LedgerAppendPlan, LedgerCheckpoint, LedgerError, LedgerEventKind,
    OutboxAdmission, TaskCreatedProfile, TaskIngressClaim, TaskIngressRequestKind,
    TaskSubmissionEnvelope, UntrustedAppendRequest, UntrustedAutonomyReceiptRow,
    UntrustedCommandReceipt, UntrustedCommandRecord, UntrustedForemanSnapshotRow,
    UntrustedLedgerEvent, UntrustedLedgerSnapshot, UntrustedOutboxAdmission,
    UntrustedTaskIngressClaim, UntrustedTaskSubmissionEnvelope, VerifiedAutonomyReceipt,
    VerifiedAutonomyReceiptState, VerifiedForemanSnapshotRecord, VerifiedStream, apply_append_plan,
    classify_task_created_profile, foreman_coordination_identity, plan_append,
    verify_untrusted_autonomy_receipt_rows, verify_untrusted_foreman_snapshot_rows,
    verify_untrusted_snapshot_against_checkpoint, verify_untrusted_task_ingress_claim,
    verify_untrusted_task_ingress_claim_structure, verify_untrusted_task_submission,
};
use postgres::error::SqlState;
use postgres::types::{FromSqlOwned, ToSql};
use postgres::{Client, Error as PostgresError, GenericClient, IsolationLevel, Row, Transaction};
use serde_json::Value as JsonValue;

use crate::postgres_setup::verify_runtime_store_schema;
use crate::{
    MigrationTarget, PostgresStoreSetupErrorKind, build_live_receipt, genesis_head, physical_head,
    request_digest, validate_physical_head,
};

const STORE_TRANSACTION_ID_SCHEMA: &str = "lattice.postgres-task-ledger.store-transaction-id";
const STORE_TRANSACTION_ID_VERSION: &str = "1.0";
const STORE_TRANSACTION_ID_PREFIX: &str = "task-ledger-v1:";
const FROZEN_STORE_SCHEMA_VERSION: u16 = 2;
const FROZEN_STORE_MANIFEST_SHA256: &str =
    "4582edce68a947998a8f4c6895bb37ceec9e842f516471f4d9e2617a6757f129";
const LEGACY_GLOBAL_LEDGER_SCHEMA_VERSION: u16 = 3;
const LEGACY_GLOBAL_LEDGER_MANIFEST_SHA256: &str =
    "09c431df18ad71a4f44239a5d2ddf6b1774b8ffec06c7f9223f0e41757f3d407";
const CURRENT_GLOBAL_LEDGER_SCHEMA_VERSION: u16 = 5;
const FOREMAN_GLOBAL_LEDGER_SCHEMA_VERSION: u16 = 6;
const SUBMISSION_GLOBAL_LEDGER_SCHEMA_VERSION: u16 = 7;
const MAX_LIVE_SERIALIZATION_RETRIES: u8 = 3;
const LOWER_HEX: &[u8; 16] = b"0123456789abcdef";
const ZERO_DIGEST_TEXT: &str = "0000000000000000000000000000000000000000000000000000000000000000";
const WRITER_LEASE_ASSERT_CURRENT_SQL: &str = "SELECT writer_lease.writer_lease_assert_current_v1(\
        $1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15)";

const LEDGER_PREPARE_V5_SQL: &str = "\
    SELECT stream_found, command_found, retained_request_digest, \
           retained_receipt_digest, retained_base_checkpoint_digest, \
           retained_result_checkpoint_digest, retained_store_transaction_id, \
           terminal_found, physical_state_digest \
      FROM control.task_ledger_prepare_v3(\
           $1::smallint, $2::text, $3::bytea, $4::text)";

const LEDGER_HEAD_V5_SQL: &str = "\
    SELECT stream_id, ledger_schema_version, head_contract_version, producer_id, \
           producer_version, runtime, project_id, project_snapshot_id, task_id, \
           task_revision, task_spec_digest, accounting_currency, sequence, \
           last_event_digest, resource_revision, resource_projection_digest, \
           head_digest, active_agents, active_implementers, elapsed_seconds, \
           attempt_number, used_model_calls, used_external_cost, retained_event_count, \
           retained_command_count, retained_outbox_count, checkpoint_schema_version, \
           checkpoint_digest, actual_event_count, actual_command_count, \
           actual_outbox_count, physical_head_found, physical_revision, \
           physical_state_digest, physical_head_digest, global_schema_version, \
           global_manifest_sha256 \
      FROM control.task_ledger_read_head_v3(\
           $1::smallint, $2::text, $3::bytea, $4::text, $5::text)";

const LEDGER_HEAD_V7_SQL: &str = "\
    SELECT stream_id, ledger_schema_version, head_contract_version, producer_id, \
           producer_version, runtime, project_id, project_snapshot_id, task_id, \
           task_revision, task_subject_kind, task_subject_digest, task_spec_digest, \
           accounting_currency, sequence, last_event_digest, resource_revision, \
           resource_projection_digest, head_digest, active_agents, active_implementers, \
           elapsed_seconds, attempt_number, used_model_calls, used_external_cost, \
           retained_event_count, retained_command_count, retained_outbox_count, \
           checkpoint_schema_version, checkpoint_digest, actual_event_count, \
           actual_command_count, actual_outbox_count, physical_head_found, \
           physical_revision, physical_state_digest, physical_head_digest, \
           global_schema_version, global_manifest_sha256 \
      FROM control.task_ledger_read_head_v4(\
           $1::smallint, $2::text, $3::bytea, $4::text, $5::text)";

const LEDGER_EVENTS_V5_SQL: &str = "\
    SELECT stream_id, event_sequence, event_schema_version, previous_event_digest, \
           command_id, request_digest, correlation_id, occurred_at, event_kind, \
           actor_id, action_id, audit_outcome, reason_code, subject_digest, diagnostic, \
           has_resource_snapshot, resource_active_agents, resource_active_implementers, \
           resource_elapsed_seconds, resource_attempt_number, resource_used_model_calls, \
           resource_used_external_cost, resource_revision, resource_projection_digest, \
           event_digest, outbox_found, admission_digest, admission_schema_version, \
           admission_state, admission_intent_digest, admission_occurred_at \
      FROM control.task_ledger_read_events_v3($1::smallint, $2::text, $3::bytea)";

const LEDGER_COMMANDS_V5_SQL: &str = "\
    SELECT stream_id, command_id, request_schema_version, request_digest, \
           expected_sequence, expected_last_event_digest, expected_resource_revision, \
           expected_resource_projection_digest, expected_head_digest, correlation_id, \
           occurred_at, event_kind, actor_id, action_id, audit_outcome, reason_code, \
           subject_digest, diagnostic, has_resource_snapshot, resource_active_agents, \
           resource_active_implementers, resource_elapsed_seconds, resource_attempt_number, \
           resource_used_model_calls, resource_used_external_cost, receipt_schema_version, \
           before_sequence, before_last_event_digest, before_resource_revision, \
           before_resource_projection_digest, before_head_digest, after_sequence, \
           after_last_event_digest, after_resource_revision, after_resource_projection_digest, \
           after_head_digest, command_outcome, denial_reason, event_digest, \
           command_receipt_digest, base_checkpoint_digest, result_checkpoint_digest, \
           command_record_set_digest, store_transaction_id, store_found, \
           store_contract_version, store_producer_id, store_producer_version, store_runtime, \
           store_durability, store_database_uuid::text, store_database_identity_digest, \
           store_schema_version, store_manifest_sha256, store_project_id, \
           store_project_snapshot_id, store_repository_owner, store_aggregate_key_digest, \
           store_request_digest, store_daemon_instance_id, store_daemon_epoch, \
           store_admission_mode, store_authority_revision, store_authority_observation_digest, \
           store_authority_head_digest, store_expected_revision, store_expected_state_digest, \
           store_expected_head_digest, store_domain_command_digest, store_record_set_digest, \
           store_next_state_digest, store_domain_receipt_digest, store_checkpoint_digest, \
           store_outbox_intent_digest, store_disposition, store_before_revision, \
           store_before_state_digest, store_before_head_digest, store_after_revision, \
           store_after_state_digest, store_after_head_digest, store_transaction_digest, \
           store_receipt_digest \
      FROM control.task_ledger_read_commands_v3($1::smallint, $2::text, $3::bytea)";

const STORE_PREPARE_V5_SQL: &str = "\
    SELECT prepare_status, database_uuid::text, database_identity_digest, schema_version, \
           manifest_sha256, head_found, before_revision, before_state_digest, \
           before_head_digest, after_revision, after_state_digest, after_head_digest, \
           terminal_disposition, terminal_transaction_digest, terminal_receipt_digest, \
           global_schema_version, global_manifest_sha256 \
      FROM control.store_prepare_v5(\
           $1::smallint, $2::text, $3::smallint, $4::text, $5::text, $6::text, \
           $7::text, $8::bytea, $9::bytea, $10::text, $11::text, $12::bigint, \
           $13::text, $14::bigint, $15::bytea, $16::bytea, $17::text, $18::bigint, \
           $19::bytea, $20::bytea, $21::bytea, $22::bytea, $23::bytea, $24::bytea, \
           $25::bytea, $26::bytea, $27::bytea, $28::bytea)";

const STORE_FINALIZE_V5_SQL: &str = "\
    SELECT control.store_finalize_v5(\
           $1::smallint, $2::text, $3::smallint, $4::text, $5::text, $6::text, \
           $7::text, $8::bytea, $9::bytea, $10::text, $11::text, $12::bigint, \
           $13::text, $14::bigint, $15::bytea, $16::bytea, $17::text, $18::bigint, \
           $19::bytea, $20::bytea, $21::bytea, $22::bytea, $23::bytea, $24::bytea, \
           $25::bytea, $26::bytea, $27::bytea, $28::bytea, $29::text::uuid, \
           $30::bytea, $31::smallint, $32::text, $33::bigint, $34::bytea, \
           $35::bytea, $36::bigint, $37::bytea, $38::bytea, $39::text, \
           $40::bytea, $41::bytea)";

const STORE_CURRENT_V5_SQL: &str = "\
    SELECT database_uuid::text, schema_version, manifest_sha256, head_found, \
           physical_revision, state_digest, head_digest, global_schema_version, \
           global_manifest_sha256 \
      FROM control.store_current_head_v5(\
           $1::smallint, $2::text, $3::text, $4::text, $5::text, $6::bytea)";

const LEDGER_FINALIZE_V5_SQL: &str = "\
    SELECT control.task_ledger_finalize_v3(\
           $1::smallint, $2::text, $3::bytea, $4::text, $5::text, $6::text, \
           $7::text, $8::bytea, $9::text, $10::text, $11::bytea, $12::text, \
           $13::bytea, $14::bytea, $15::text, $16::text, $17::text, $18::text, \
           $19::text, $20::text, $21::text, $22::text, $23::text, $24::bytea, \
           $25::bytea, $26::text, $27::bytea, $28::text, $29::bytea, $30::text, \
           $31::bytea, $32::bytea, $33::text, $34::text, $35::text, $36::text, \
           $37::text, $38::text, $39::text, $40::bytea, $41::jsonb, $42::boolean, \
           $43::text, $44::text, $45::text, $46::text, $47::text, $48::text, \
           $49::text, $50::bytea, $51::text, $52::bytea, $53::bytea, $54::text, \
           $55::bytea, $56::text, $57::bytea, $58::bytea, $59::text, $60::text, \
           $61::bytea, $62::bytea, $63::bytea, $64::text, $65::boolean, $66::text, \
           $67::bytea, $68::text, $69::bytea, $70::boolean, $71::bytea, $72::bytea)";

const LEDGER_FINALIZE_GENERAL_V7_SQL: &str = "\
    SELECT control.task_ledger_finalize_general_intake_v1(\
           $1::smallint,$2::text,$3::bytea,$4::text,$5::text,$6::text,\
           $7::text,$8::text,$9::bytea,$10::text,$11::bytea,$12::text,\
           $13::bytea,$14::bytea,$15::text,$16::text,$17::text,$18::text,\
           $19::text,$20::text,$21::text,$22::text,$23::text,$24::bytea,\
           $25::bytea,$26::text,$27::bytea,$28::text,$29::bytea,$30::text,\
           $31::bytea,$32::bytea,$33::text,$34::text,$35::text,$36::bytea,\
           $37::text,$38::bytea,$39::text,$40::bytea,$41::bytea,$42::text,\
           $43::bytea,$44::text,$45::bytea,$46::bytea,$47::bytea,$48::bytea,\
           $49::bytea,$50::text,$51::text,$52::bytea,$53::text,$54::bytea)";

const LEDGER_PREPARE_V3_SQL: &str = "\
    SELECT stream_found, command_found, retained_request_digest, \
           retained_receipt_digest, retained_base_checkpoint_digest, \
           retained_result_checkpoint_digest, retained_store_transaction_id, \
           terminal_found, physical_state_digest \
      FROM control.task_ledger_prepare_v1($1::bytea, $2::text)";

const LEDGER_HEAD_V3_SQL: &str = "\
    SELECT stream_id, ledger_schema_version, head_contract_version, producer_id, \
           producer_version, runtime, project_id, project_snapshot_id, task_id, \
           task_revision, task_spec_digest, accounting_currency, sequence, \
           last_event_digest, resource_revision, resource_projection_digest, \
           head_digest, active_agents, active_implementers, elapsed_seconds, \
           attempt_number, used_model_calls, used_external_cost, retained_event_count, \
           retained_command_count, retained_outbox_count, checkpoint_schema_version, \
           checkpoint_digest, actual_event_count, actual_command_count, \
           actual_outbox_count, physical_head_found, physical_revision, \
           physical_state_digest, physical_head_digest, global_schema_version, \
           global_manifest_sha256 \
      FROM control.task_ledger_read_head_v1($1::bytea, $2::text, $3::text)";

const LEDGER_EVENTS_V3_SQL: &str = "\
    SELECT stream_id, event_sequence, event_schema_version, previous_event_digest, \
           command_id, request_digest, correlation_id, occurred_at, event_kind, \
           actor_id, action_id, audit_outcome, reason_code, subject_digest, diagnostic, \
           has_resource_snapshot, resource_active_agents, resource_active_implementers, \
           resource_elapsed_seconds, resource_attempt_number, resource_used_model_calls, \
           resource_used_external_cost, resource_revision, resource_projection_digest, \
           event_digest, outbox_found, admission_digest, admission_schema_version, \
           admission_state, admission_intent_digest, admission_occurred_at \
      FROM control.task_ledger_read_events_v1($1::bytea)";

const LEDGER_COMMANDS_V3_SQL: &str = "\
    SELECT stream_id, command_id, request_schema_version, request_digest, \
           expected_sequence, expected_last_event_digest, expected_resource_revision, \
           expected_resource_projection_digest, expected_head_digest, correlation_id, \
           occurred_at, event_kind, actor_id, action_id, audit_outcome, reason_code, \
           subject_digest, diagnostic, has_resource_snapshot, resource_active_agents, \
           resource_active_implementers, resource_elapsed_seconds, resource_attempt_number, \
           resource_used_model_calls, resource_used_external_cost, receipt_schema_version, \
           before_sequence, before_last_event_digest, before_resource_revision, \
           before_resource_projection_digest, before_head_digest, after_sequence, \
           after_last_event_digest, after_resource_revision, after_resource_projection_digest, \
           after_head_digest, command_outcome, denial_reason, event_digest, \
           command_receipt_digest, base_checkpoint_digest, result_checkpoint_digest, \
           command_record_set_digest, store_transaction_id, store_found, \
           store_contract_version, store_producer_id, store_producer_version, store_runtime, \
           store_durability, store_database_uuid::text, store_database_identity_digest, \
           store_schema_version, store_manifest_sha256, store_project_id, \
           store_project_snapshot_id, store_repository_owner, store_aggregate_key_digest, \
           store_request_digest, store_daemon_instance_id, store_daemon_epoch, \
           store_admission_mode, store_authority_revision, store_authority_observation_digest, \
           store_authority_head_digest, store_expected_revision, store_expected_state_digest, \
           store_expected_head_digest, store_domain_command_digest, store_record_set_digest, \
           store_next_state_digest, store_domain_receipt_digest, store_checkpoint_digest, \
           store_outbox_intent_digest, store_disposition, store_before_revision, \
           store_before_state_digest, store_before_head_digest, store_after_revision, \
           store_after_state_digest, store_after_head_digest, store_transaction_digest, \
           store_receipt_digest \
      FROM control.task_ledger_read_commands_v1($1::bytea)";

const LEDGER_AUTONOMY_RECEIPTS_SQL: &str = "\
    SELECT stream_id, event_sequence, event_digest, receipt_schema_version, \
           intent_version, task_kind, risk_class, execution_preapproved, \
           requires_new_authority, irreversible_or_high_risk, observed_task_state, \
           disposition, decision_reason, model, verification, authority_mode, \
           process_start_authority_digest, ingress_profile_adapter_commitment, \
           store_authority_head_digest, writer_lease_receipt_digest, \
           writer_lease_head_digest, writer_fencing_token, authority_digest, receipt_digest \
      FROM control.task_ledger_read_autonomy_receipts_v1($1::bytea)";

const LEDGER_RECORD_AUTONOMY_RECEIPT_SQL: &str = "\
    SELECT control.task_ledger_record_autonomy_receipt_v1(\
        $1::bytea,$2::text,$3::bytea,$4::text,$5::text,$6::text,$7::text,\
        $8::boolean,$9::boolean,$10::boolean,$11::text,$12::text,$13::text,\
        $14::text,$15::text,$16::text,$17::bytea,$18::bytea,$19::bytea,\
        $20::bytea,$21::bytea,$22::text,$23::bytea,$24::bytea)";

const LEDGER_FOREMAN_SNAPSHOTS_SQL: &str = "\
    SELECT stream_id,event_sequence,event_digest,command_id,request_digest,record_schema,\
           payload_schema,payload_digest,worker_id,thread_id,task_id,branch_ref,worktree_ref,\
           head_sha1,foreman_state,blocker_ref,heartbeat_digest_ref,authority_digest_ref,\
           evidence_digest_ref,generation,epistemic_schema,observed_fact_refs,hypothesis_refs,confidence,\
           unknown_refs,evidence_refs,counterevidence_refs,checked_at,expires_at,\
           refresh_trigger,decision_ref,probe_ref,falsifier_ref \
      FROM control.task_ledger_read_foreman_snapshots_v1($1::bytea)";

const LEDGER_RECORD_FOREMAN_SNAPSHOT_SQL: &str = "\
    SELECT control.task_ledger_record_foreman_snapshot_v1(\
        $1::text,$2::text,$3::text,$4::text,$5::bytea,$6::text,$7::text,$8::text,\
        $9::text,$10::bigint,$11::bytea,$12::text,$13::bigint,$14::bigint,$15::bytea,\
        $16::bytea,$17::text,$18::bytea,$19::text,$20::bytea,$21::text,$22::text,\
        $23::bytea,$24::text,$25::text,$26::text,$27::text,$28::text,$29::text,\
        $30::text,$31::text,$32::text,$33::text,$34::text,$35::text,$36::text,\
        $37::text[],$38::text[],$39::text,$40::text[],$41::text[],$42::text[],\
        $43::text,$44::text,$45::text,$46::text,$47::text,$48::text)";

const TASK_INGRESS_PREPARE_SQL: &str = "\
    SELECT found,schema_version,ingress_id,client_request_id,request_kind,\
           ingress_request_digest,stream_id,event_sequence,event_digest,command_id,\
           command_request_digest,event_kind,event_action,event_audit_outcome \
      FROM control.task_ingress_prepare_v1($1::text,$2::text,$3::text,$4::bytea,$5::bytea)";

const TASK_INGRESS_RECORD_SQL: &str = "\
    SELECT control.task_ingress_record_v1(\
        $1::text,$2::text,$3::text,$4::text,$5::bytea,$6::bytea,$7::text,\
        $8::bytea,$9::text,$10::bytea)";

const TASK_INGRESS_READ_BY_REQUEST_SQL: &str = "\
    SELECT schema_version,ingress_id,client_request_id,request_kind,\
           ingress_request_digest,stream_id,event_sequence,event_digest,command_id,\
           command_request_digest,event_kind,event_action,event_audit_outcome \
      FROM control.task_ingress_read_by_request_v1($1::text,$2::text)";

const TASK_SUBMISSION_PREPARE_SQL: &str = "\
    SELECT found,schema_version,ingress_id,client_request_id,objective,project_display_name,\
           project_authority_receipt_digest,project_id,project_snapshot_id,task_id,task_revision,\
           task_subject_kind,intake_digest,stream_id,task_ref,admission_action,envelope_digest,\
           event_sequence,event_digest,command_id,request_digest,ingress_request_digest \
      FROM control.task_submission_prepare_v1($1::text,$2::text,$3::bytea)";

const TASK_SUBMISSION_RECORD_SQL: &str = "\
    SELECT control.task_submission_record_v1(\
        $1::text,$2::text,$3::text,$4::text,$5::text,$6::bytea,$7::text,$8::text,\
        $9::text,$10::text,$11::text,$12::bytea,$13::bytea,$14::text,$15::text,\
        $16::bytea,$17::text,$18::bytea,$19::text,$20::bytea,$21::bytea)";

const TASK_SUBMISSION_READ_BY_REF_SQL: &str = "\
    SELECT schema_version,ingress_id,client_request_id,objective,project_display_name,\
           project_authority_receipt_digest,project_id,project_snapshot_id,task_id,task_revision,\
           task_subject_kind,intake_digest,stream_id,task_ref,admission_action,envelope_digest,\
           event_sequence,event_digest,command_id,request_digest,ingress_request_digest \
      FROM control.task_submission_read_by_task_ref_v1($1::text)";

const TASK_SUBMISSION_READ_BY_REQUEST_SQL: &str = "\
    SELECT schema_version,ingress_id,client_request_id,objective,project_display_name,\
           project_authority_receipt_digest,project_id,project_snapshot_id,task_id,task_revision,\
           task_subject_kind,intake_digest,stream_id,task_ref,admission_action,envelope_digest,\
           event_sequence,event_digest,command_id,request_digest,ingress_request_digest \
      FROM control.task_submission_read_by_request_v1($1::text,$2::text)";

const STORE_PREPARE_V3_SQL: &str = "\
    SELECT prepare_status, database_uuid::text, database_identity_digest, schema_version, \
           manifest_sha256, head_found, before_revision, before_state_digest, \
           before_head_digest, after_revision, after_state_digest, after_head_digest, \
           terminal_disposition, terminal_transaction_digest, terminal_receipt_digest, \
           global_schema_version, global_manifest_sha256 \
      FROM control.store_prepare_v3(\
           $1::smallint, $2::text, $3::text, $4::text, $5::text, $6::bytea, \
           $7::bytea, $8::text, $9::text, $10::bigint, $11::text, $12::bigint, \
           $13::bytea, $14::bytea, $15::text, $16::bigint, $17::bytea, $18::bytea, \
           $19::bytea, $20::bytea, $21::bytea, $22::bytea, $23::bytea, $24::bytea, \
           $25::bytea, $26::bytea)";

const STORE_FINALIZE_V3_SQL: &str = "\
    SELECT control.store_finalize_v3(\
           $1::smallint, $2::text, $3::text, $4::text, $5::text, $6::bytea, \
           $7::bytea, $8::text, $9::text, $10::bigint, $11::text, $12::bigint, \
           $13::bytea, $14::bytea, $15::text, $16::bigint, $17::bytea, $18::bytea, \
           $19::bytea, $20::bytea, $21::bytea, $22::bytea, $23::bytea, $24::bytea, \
           $25::bytea, $26::bytea, $27::text::uuid, $28::bytea, $29::smallint, \
           $30::text, $31::bigint, $32::bytea, $33::bytea, $34::bigint, $35::bytea, \
           $36::bytea, $37::text, $38::bytea, $39::bytea)";

const STORE_CURRENT_V3_SQL: &str = "\
    SELECT database_uuid::text, schema_version, manifest_sha256, head_found, \
           physical_revision, state_digest, head_digest, global_schema_version, \
           global_manifest_sha256 \
      FROM control.store_current_head_v3($1::text, $2::text, $3::text, $4::bytea)";

const LEDGER_FINALIZE_V3_SQL: &str = "\
    SELECT control.task_ledger_finalize_v1(\
           $1::bytea, $2::text, $3::text, $4::text, $5::text, $6::bytea, $7::text, \
           $8::text, $9::bytea, $10::text, $11::bytea, $12::bytea, $13::text, \
           $14::text, $15::text, $16::text, $17::text, $18::text, $19::text, \
           $20::text, $21::text, $22::bytea, $23::bytea, $24::text, $25::bytea, \
           $26::text, $27::bytea, $28::text, $29::bytea, $30::bytea, $31::text, \
           $32::text, $33::text, $34::text, $35::text, $36::text, $37::text, \
           $38::bytea, $39::jsonb, $40::boolean, $41::text, $42::text, $43::text, \
           $44::text, $45::text, $46::text, $47::text, $48::bytea, $49::text, \
           $50::bytea, $51::bytea, $52::text, $53::bytea, $54::text, $55::bytea, \
           $56::bytea, $57::text, $58::text, $59::bytea, $60::bytea, $61::bytea, \
           $62::text, $63::boolean, $64::text, $65::bytea, $66::text, $67::bytea, \
           $68::boolean, $69::bytea, $70::bytea)";

const WRITE_TRANSACTION_SETTINGS: &str = "\
    SET LOCAL search_path = pg_catalog; \
    SET LOCAL row_security = on; \
    SET LOCAL synchronous_commit = on; \
    SET LOCAL lock_timeout = '5s'; \
    SET LOCAL statement_timeout = '30s'";

const READ_TRANSACTION_SETTINGS: &str = "\
    SET LOCAL search_path = pg_catalog; \
    SET LOCAL row_security = on; \
    SET LOCAL lock_timeout = '5s'; \
    SET LOCAL statement_timeout = '30s'";

/// Result returned by the live durable Task Ledger adapter.
pub type PostgresTaskLedgerResult<T> = Result<T, PostgresTaskLedgerError>;

/// Static fail-closed error categories for the live Task Ledger adapter.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PostgresTaskLedgerErrorKind {
    Malformed,
    CommandSubstitution,
    ProjectRegistryCurrentnessConflict,
    ProjectRegistryInactive,
    AdmissionDenied,
    AuthorityMismatch,
    PhysicalStateMismatch,
    CheckpointCorrupt,
    RetainedRowCorrupt,
    UnsupportedRetainedSchema,
    RevisionOverflow,
    SerializationExhausted,
    TransactionFailed,
    Unavailable,
    CommitOutcomeUnknown,
}

impl PostgresTaskLedgerErrorKind {
    /// Complete closed error set.
    pub const ALL: [Self; 15] = [
        Self::Malformed,
        Self::CommandSubstitution,
        Self::ProjectRegistryCurrentnessConflict,
        Self::ProjectRegistryInactive,
        Self::AdmissionDenied,
        Self::AuthorityMismatch,
        Self::PhysicalStateMismatch,
        Self::CheckpointCorrupt,
        Self::RetainedRowCorrupt,
        Self::UnsupportedRetainedSchema,
        Self::RevisionOverflow,
        Self::SerializationExhausted,
        Self::TransactionFailed,
        Self::Unavailable,
        Self::CommitOutcomeUnknown,
    ];

    /// Stable machine-facing code with no database diagnostic.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Malformed => "POSTGRES_TASK_LEDGER_MALFORMED",
            Self::CommandSubstitution => "POSTGRES_TASK_LEDGER_COMMAND_SUBSTITUTED",
            Self::ProjectRegistryCurrentnessConflict => {
                "POSTGRES_TASK_LEDGER_PROJECT_REGISTRY_CURRENTNESS_CONFLICT"
            }
            Self::ProjectRegistryInactive => "POSTGRES_TASK_LEDGER_PROJECT_REGISTRY_INACTIVE",
            Self::AdmissionDenied => "POSTGRES_TASK_LEDGER_ADMISSION_DENIED",
            Self::AuthorityMismatch => "POSTGRES_TASK_LEDGER_AUTHORITY_MISMATCH",
            Self::PhysicalStateMismatch => "POSTGRES_TASK_LEDGER_PHYSICAL_STATE_MISMATCH",
            Self::CheckpointCorrupt => "POSTGRES_TASK_LEDGER_CHECKPOINT_CORRUPT",
            Self::RetainedRowCorrupt => "POSTGRES_TASK_LEDGER_RETAINED_ROW_CORRUPT",
            Self::UnsupportedRetainedSchema => "POSTGRES_TASK_LEDGER_UNSUPPORTED_RETAINED_SCHEMA",
            Self::RevisionOverflow => "POSTGRES_TASK_LEDGER_REVISION_OVERFLOW",
            Self::SerializationExhausted => "POSTGRES_TASK_LEDGER_SERIALIZATION_EXHAUSTED",
            Self::TransactionFailed => "POSTGRES_TASK_LEDGER_TRANSACTION_FAILED",
            Self::Unavailable => "POSTGRES_TASK_LEDGER_UNAVAILABLE",
            Self::CommitOutcomeUnknown => "POSTGRES_TASK_LEDGER_COMMIT_OUTCOME_UNKNOWN",
        }
    }
}

/// Bounded static error that never retains SQL, values, credentials, or driver output.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PostgresTaskLedgerError {
    kind: PostgresTaskLedgerErrorKind,
}

impl PostgresTaskLedgerError {
    #[must_use]
    pub const fn new(kind: PostgresTaskLedgerErrorKind) -> Self {
        Self { kind }
    }

    #[must_use]
    pub const fn kind(self) -> PostgresTaskLedgerErrorKind {
        self.kind
    }

    #[must_use]
    pub const fn code(self) -> &'static str {
        self.kind.code()
    }
}

impl fmt::Display for PostgresTaskLedgerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl Error for PostgresTaskLedgerError {}

/// Global schema-v5 persistence identity, distinct from Store receipt profile 2.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PostgresTaskLedgerPersistenceEvidence {
    database_identity_digest: ContentDigest,
    schema_version: u16,
    manifest_digest: ContentDigest,
}

impl PostgresTaskLedgerPersistenceEvidence {
    #[must_use]
    pub const fn database_identity_digest(&self) -> &ContentDigest {
        &self.database_identity_digest
    }

    #[must_use]
    pub const fn schema_version(&self) -> u16 {
        self.schema_version
    }

    #[must_use]
    pub const fn manifest_digest(&self) -> &ContentDigest {
        &self.manifest_digest
    }
}

/// One fully verified durable stream observation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PostgresTaskLedgerLoad {
    stream: VerifiedStream,
    retained_checkpoint: LedgerCheckpoint,
    physical_head: StorePhysicalHead,
    persistence: PostgresTaskLedgerPersistenceEvidence,
    autonomy_state: VerifiedAutonomyReceiptState,
}

/// One repeatable-read observation of the fixed foreman Ledger stream and
/// every independently verified fixed-scalar child record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PostgresForemanReplay {
    ledger: PostgresTaskLedgerLoad,
    records: Vec<VerifiedForemanSnapshotRecord>,
}

impl PostgresForemanReplay {
    #[must_use]
    pub const fn ledger(&self) -> &PostgresTaskLedgerLoad {
        &self.ledger
    }

    #[must_use]
    pub fn records(&self) -> &[VerifiedForemanSnapshotRecord] {
        &self.records
    }
}

impl PostgresTaskLedgerLoad {
    #[must_use]
    pub const fn stream(&self) -> &VerifiedStream {
        &self.stream
    }

    #[must_use]
    pub const fn retained_checkpoint(&self) -> &LedgerCheckpoint {
        &self.retained_checkpoint
    }

    #[must_use]
    pub const fn physical_head(&self) -> &StorePhysicalHead {
        &self.physical_head
    }

    #[must_use]
    pub const fn persistence(&self) -> &PostgresTaskLedgerPersistenceEvidence {
        &self.persistence
    }

    #[must_use]
    pub const fn autonomy_state(&self) -> &VerifiedAutonomyReceiptState {
        &self.autonomy_state
    }

    #[must_use]
    pub const fn autonomy_receipt(&self) -> Option<&VerifiedAutonomyReceipt> {
        verified_autonomy_receipt(&self.autonomy_state)
    }
}

const fn verified_autonomy_receipt(
    state: &VerifiedAutonomyReceiptState,
) -> Option<&VerifiedAutonomyReceipt> {
    match state {
        VerifiedAutonomyReceiptState::HistoricalOptional(Some(receipt))
        | VerifiedAutonomyReceiptState::RequiredComplete(receipt) => Some(receipt),
        VerifiedAutonomyReceiptState::NotApplicable
        | VerifiedAutonomyReceiptState::HistoricalOptional(None)
        | VerifiedAutonomyReceiptState::PendingRequiredReceipt => None,
    }
}

/// Durable terminal result returned only after a known successful commit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PostgresTaskLedgerExecution {
    receipt: CommandReceipt,
    result_checkpoint: LedgerCheckpoint,
    outbox_admission: Option<OutboxAdmission>,
    store_receipt: StoreTransactionReceipt,
    persistence: PostgresTaskLedgerPersistenceEvidence,
    exact_retry: bool,
}

impl PostgresTaskLedgerExecution {
    #[must_use]
    pub const fn receipt(&self) -> &CommandReceipt {
        &self.receipt
    }

    #[must_use]
    pub const fn result_checkpoint(&self) -> &LedgerCheckpoint {
        &self.result_checkpoint
    }

    #[must_use]
    pub const fn outbox_admission(&self) -> Option<&OutboxAdmission> {
        self.outbox_admission.as_ref()
    }

    #[must_use]
    pub const fn store_receipt(&self) -> &StoreTransactionReceipt {
        &self.store_receipt
    }

    #[must_use]
    pub const fn persistence(&self) -> &PostgresTaskLedgerPersistenceEvidence {
        &self.persistence
    }

    #[must_use]
    pub const fn is_exact_retry(&self) -> bool {
        self.exact_retry
    }
}

/// Atomic result of one authoritative general-task submission append.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PostgresTaskSubmissionExecution {
    ledger_execution: PostgresTaskLedgerExecution,
    submission: TaskSubmissionEnvelope,
}

impl PostgresTaskSubmissionExecution {
    /// Returns the durable Ledger/Store execution evidence.
    #[must_use]
    pub const fn ledger_execution(&self) -> &PostgresTaskLedgerExecution {
        &self.ledger_execution
    }

    /// Returns the replay-verified authoritative submission envelope.
    #[must_use]
    pub const fn submission(&self) -> &TaskSubmissionEnvelope {
        &self.submission
    }
}

/// Fresh-process load of an authoritative submission plus its verified stream.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PostgresTaskSubmissionLoad {
    submission: TaskSubmissionEnvelope,
    ledger: PostgresTaskLedgerLoad,
}

impl PostgresTaskSubmissionLoad {
    /// Returns the replay-verified authoritative submission envelope.
    #[must_use]
    pub const fn submission(&self) -> &TaskSubmissionEnvelope {
        &self.submission
    }

    /// Returns the complete replay-verified Task Ledger stream.
    #[must_use]
    pub const fn ledger(&self) -> &PostgresTaskLedgerLoad {
        &self.ledger
    }
}

/// Synchronous live Task Ledger adapter over one authenticated runtime client.
pub struct PostgresTaskLedger {
    client: Client,
    sql_profile: TaskLedgerSqlProfile,
    global_persistence: PostgresTaskLedgerPersistenceEvidence,
    store_receipt_persistence: StorePersistenceEvidence,
    database_uuid: String,
    commit_outcome_unknown: bool,
}

impl PostgresTaskLedger {
    /// Verifies an exact frozen schema-v3, schema-v5, foreman schema-v6, or
    /// general-submission schema-v7 runtime surface before accepting the client.
    ///
    /// # Errors
    ///
    /// Fails closed unless the target, global schema, and frozen Store receipt
    /// persistence profile all verify exactly.
    pub fn new(mut client: Client, target: &MigrationTarget) -> PostgresTaskLedgerResult<Self> {
        let evidence = verify_runtime_store_schema(&mut client, target).map_err(map_setup_error)?;
        let sql_profile = global_ledger_sql_profile(evidence.global_schema_version())
            .ok_or_else(|| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))?;
        if (sql_profile == TaskLedgerSqlProfile::V3
            && evidence.global_manifest_sha256().as_str() != LEGACY_GLOBAL_LEDGER_MANIFEST_SHA256)
            || evidence.schema_version() != FROZEN_STORE_SCHEMA_VERSION
            || evidence.manifest_sha256().as_str() != FROZEN_STORE_MANIFEST_SHA256
        {
            return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt));
        }
        let database_identity_digest = digest(target.expected_database_identity_sha256().as_str())?;
        let global_manifest = digest(evidence.global_manifest_sha256().as_str())?;
        let frozen_manifest = digest(evidence.manifest_sha256().as_str())?;
        let global_persistence = PostgresTaskLedgerPersistenceEvidence {
            database_identity_digest: database_identity_digest.clone(),
            schema_version: evidence.global_schema_version(),
            manifest_digest: global_manifest,
        };
        let store_receipt_persistence = StorePersistenceEvidence::new(
            database_identity_digest,
            FROZEN_STORE_SCHEMA_VERSION,
            frozen_manifest,
        )
        .map_err(|_| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))?;
        Ok(Self {
            client,
            sql_profile,
            global_persistence,
            store_receipt_persistence,
            database_uuid: evidence.database_uuid().to_owned(),
            commit_outcome_unknown: false,
        })
    }

    /// Loads and verifies one exact stream through the fixed repository surface.
    ///
    /// # Errors
    ///
    /// Fails closed on unavailable persistence, malformed retained rows,
    /// checkpoint disagreement, or a physical Store mismatch.
    #[allow(clippy::needless_pass_by_value)]
    pub fn load_stream(
        &mut self,
        identity: TaskLedgerStreamIdentity,
    ) -> PostgresTaskLedgerResult<PostgresTaskLedgerLoad> {
        self.ensure_reconcilable()?;
        let stream_id = VerifiedStream::vacant(identity.clone(), RuntimeKind::Live)
            .map_err(|ledger| map_ledger_error(&ledger))?
            .head()
            .stream_id()
            .clone();
        let stream_id_bytes = digest_bytes(&stream_id)?;
        let mut transaction = self
            .client
            .build_transaction()
            .isolation_level(IsolationLevel::RepeatableRead)
            .read_only(true)
            .start()
            .map_err(|database| map_database_error(&database))?;
        if let Err(database) = transaction.batch_execute(READ_TRANSACTION_SETTINGS) {
            return rollback_load(transaction, map_database_error(&database));
        }
        let loaded = match load_verified_stream(
            &mut transaction,
            &identity,
            &stream_id_bytes,
            &self.database_uuid,
            &self.store_receipt_persistence,
            &self.global_persistence,
            self.sql_profile,
        ) {
            Ok(loaded) => loaded,
            Err(load_error) => return rollback_load(transaction, load_error),
        };
        transaction
            .commit()
            .map_err(|database| map_database_error(&database))?;
        Ok(PostgresTaskLedgerLoad {
            stream: loaded.stream,
            retained_checkpoint: loaded.retained_checkpoint,
            physical_head: loaded.physical_head,
            persistence: self.global_persistence.clone(),
            autonomy_state: loaded.autonomy_state,
        })
    }

    /// Executes one pure Ledger plan and commits its Store/Ledger rows atomically.
    ///
    /// # Errors
    ///
    /// Fails closed on malformed work, changed command reuse, rejected
    /// admission, authority/checkpoint disagreement, transaction failure, or
    /// an unknown commit outcome.
    #[allow(clippy::needless_pass_by_value)]
    pub fn execute(
        &mut self,
        command: AppendCommand,
        expected_authority: StoreAuthorityHead,
    ) -> PostgresTaskLedgerResult<PostgresTaskLedgerExecution> {
        self.execute_with_writer_authority(
            &command,
            &expected_authority,
            None,
            None,
            None,
            None,
            None,
        )
    }

    /// Appends one controlled-canary `TASK_CREATED` event under the shared,
    /// Task-Ledger-owned task-ingress idempotency keyspace.
    ///
    /// # Errors
    ///
    /// Fails closed on an invalid claim/command binding or any conflicting
    /// canary/general claim for the same ingress client request.
    #[allow(clippy::needless_pass_by_value)]
    pub fn execute_task_ingress(
        &mut self,
        command: AppendCommand,
        expected_authority: StoreAuthorityHead,
        ingress_claim: TaskIngressClaim,
    ) -> PostgresTaskLedgerResult<PostgresTaskLedgerExecution> {
        self.execute_with_writer_authority(
            &command,
            &expected_authority,
            None,
            None,
            None,
            None,
            Some(&ingress_claim),
        )
    }

    /// Appends one general `TASK_CREATED` event and its authoritative intake
    /// envelope in the same serializable Store/Ledger transaction.
    ///
    /// # Errors
    ///
    /// Exact retry returns the retained envelope; changed reuse of the same
    /// ingress/client request key returns `CommandSubstitution`. Every malformed
    /// or non-general binding fails before persistence.
    #[allow(clippy::needless_pass_by_value)]
    pub fn execute_submission(
        &mut self,
        command: AppendCommand,
        expected_authority: StoreAuthorityHead,
        submission: TaskSubmissionEnvelope,
    ) -> PostgresTaskLedgerResult<PostgresTaskSubmissionExecution> {
        let ingress_claim = TaskIngressClaim::general_submission(&submission)
            .map_err(|ledger| map_ledger_error(&ledger))?;
        let ledger_execution = self.execute_with_writer_authority(
            &command,
            &expected_authority,
            None,
            None,
            None,
            Some(&submission),
            Some(&ingress_claim),
        )?;
        Ok(PostgresTaskSubmissionExecution {
            ledger_execution,
            submission,
        })
    }

    /// Loads the structurally verified ingress claim that already owns one
    /// scoped client request key, without resolving a new task project.
    ///
    /// This is a preflight identity lookup only. For a general-task claim the
    /// request digest remains opaque until the authoritative submission
    /// envelope is loaded and verified. Callers may use the returned closed
    /// request kind to reject a cross-kind retry, but must not infer an exact
    /// general-task replay from this result alone.
    ///
    /// # Errors
    ///
    /// Fails closed on malformed lookup keys, unsupported schema, a missing or
    /// mismatched linked `TASK_CREATED` event, an invalid command/digest, or
    /// retained request-kind/action disagreement.
    pub fn load_ingress_claim_by_request(
        &mut self,
        ingress_id: &str,
        client_request_id: &str,
    ) -> PostgresTaskLedgerResult<Option<TaskIngressClaim>> {
        if !valid_submission_lookup_id(ingress_id)
            || !valid_task_ingress_client_request_id(client_request_id)
        {
            return Err(error(PostgresTaskLedgerErrorKind::Malformed));
        }
        self.ensure_reconcilable()?;
        if !self.sql_profile.supports_submission() {
            return Err(error(
                PostgresTaskLedgerErrorKind::UnsupportedRetainedSchema,
            ));
        }
        let mut transaction = self
            .client
            .build_transaction()
            .isolation_level(IsolationLevel::RepeatableRead)
            .read_only(true)
            .start()
            .map_err(|database| map_database_error(&database))?;
        if let Err(database) = transaction.batch_execute(READ_TRANSACTION_SETTINGS) {
            return rollback_load(transaction, map_database_error(&database));
        }
        let row = match transaction.query_opt(
            TASK_INGRESS_READ_BY_REQUEST_SQL,
            &[&ingress_id, &client_request_id],
        ) {
            Ok(row) => row,
            Err(database) => return rollback_load(transaction, map_database_error(&database)),
        };
        let Some(row) = row else {
            transaction
                .commit()
                .map_err(|database| map_database_error(&database))?;
            return Ok(None);
        };
        let retained = match parse_task_ingress_claim_row(&row, 0, None) {
            Ok(retained) => retained,
            Err(load_error) => return rollback_load(transaction, load_error),
        };
        if retained.claim.ingress_id() != ingress_id
            || retained.claim.client_request_id() != client_request_id
        {
            return rollback_load(
                transaction,
                error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt),
            );
        }
        transaction
            .commit()
            .map_err(|database| map_database_error(&database))?;
        Ok(Some(retained.claim))
    }

    /// Executes one append while asserting an exact current live Writer Lease
    /// inside the same serializable transaction as the Store/Ledger mutation.
    /// Exact retries remain read-only and return their retained receipt without
    /// requiring a lease that may already have been released.
    ///
    /// # Errors
    ///
    /// Fails closed on every ordinary append error plus any cross-bound,
    /// stale, inactive, or physically mismatched Writer Lease authority.
    #[allow(clippy::needless_pass_by_value)]
    pub fn execute_fenced(
        &mut self,
        command: AppendCommand,
        expected_authority: StoreAuthorityHead,
        writer_authority: WriterLeaseAuthorityHead,
    ) -> PostgresTaskLedgerResult<PostgresTaskLedgerExecution> {
        self.execute_with_writer_authority(
            &command,
            &expected_authority,
            Some(&writer_authority),
            None,
            None,
            None,
            None,
        )
    }

    /// Atomically records one Task-Ledger-planned autonomy receipt and event.
    ///
    /// # Errors
    ///
    /// Fails closed on ordinary append errors, authority mismatch, a stale or
    /// substituted typed plan, or any non-atomic retained-row disagreement.
    #[allow(clippy::needless_pass_by_value)]
    pub fn execute_autonomy(
        &mut self,
        autonomy_plan: AutonomyReceiptAppendPlan,
        expected_authority: StoreAuthorityHead,
    ) -> PostgresTaskLedgerResult<PostgresTaskLedgerExecution> {
        if !autonomy_plan_matches_store_authority(&autonomy_plan, &expected_authority) {
            return Err(error(PostgresTaskLedgerErrorKind::AuthorityMismatch));
        }
        let command = autonomy_plan
            .append_plan()
            .command_record()
            .request()
            .clone();
        self.execute_with_writer_authority(
            &command,
            &expected_authority,
            autonomy_plan.writer_authority(),
            Some(&autonomy_plan),
            None,
            None,
            None,
        )
    }

    /// Loads one authoritative submission by durable public task reference.
    ///
    /// # Errors
    ///
    /// Fails closed on unsupported schema, malformed retained envelope fields,
    /// or disagreement with the replay-verified Task Ledger stream.
    pub fn load_submission_by_task_ref(
        &mut self,
        task_ref: &ContentDigest,
    ) -> PostgresTaskLedgerResult<Option<PostgresTaskSubmissionLoad>> {
        if is_zero_content_digest(task_ref) {
            return Err(error(PostgresTaskLedgerErrorKind::Malformed));
        }
        let task_ref_text = task_ref.as_str();
        self.load_submission_with_query(TASK_SUBMISSION_READ_BY_REF_SQL, &[&task_ref_text])
    }

    /// Loads one authoritative submission by its scoped client idempotency key.
    ///
    /// # Errors
    ///
    /// Fails closed on malformed lookup values, unsupported schema, retained
    /// corruption, or disagreement with the verified Ledger stream.
    pub fn load_submission_by_request(
        &mut self,
        ingress_id: &str,
        client_request_id: &str,
    ) -> PostgresTaskLedgerResult<Option<PostgresTaskSubmissionLoad>> {
        if !valid_submission_lookup_id(ingress_id)
            || !valid_task_ingress_client_request_id(client_request_id)
        {
            return Err(error(PostgresTaskLedgerErrorKind::Malformed));
        }
        self.load_submission_with_query(
            TASK_SUBMISSION_READ_BY_REQUEST_SQL,
            &[&ingress_id, &client_request_id],
        )
    }

    fn load_submission_with_query(
        &mut self,
        sql: &str,
        params: &[&(dyn ToSql + Sync)],
    ) -> PostgresTaskLedgerResult<Option<PostgresTaskSubmissionLoad>> {
        self.ensure_reconcilable()?;
        if !self.sql_profile.supports_submission() {
            return Err(error(
                PostgresTaskLedgerErrorKind::UnsupportedRetainedSchema,
            ));
        }
        let mut transaction = self
            .client
            .build_transaction()
            .isolation_level(IsolationLevel::RepeatableRead)
            .read_only(true)
            .start()
            .map_err(|database| map_database_error(&database))?;
        if let Err(database) = transaction.batch_execute(READ_TRANSACTION_SETTINGS) {
            return rollback_load(transaction, map_database_error(&database));
        }
        let row = match transaction.query_opt(sql, params) {
            Ok(row) => row,
            Err(database) => return rollback_load(transaction, map_database_error(&database)),
        };
        let Some(row) = row else {
            transaction
                .commit()
                .map_err(|database| map_database_error(&database))?;
            return Ok(None);
        };
        let retained = match parse_submission_row(&row, 0) {
            Ok(retained) => retained,
            Err(load_error) => return rollback_load(transaction, load_error),
        };
        let expected_claim = match TaskIngressClaim::general_submission(&retained.submission) {
            Ok(claim) => claim,
            Err(ledger) => return rollback_load(transaction, map_ledger_error(&ledger)),
        };
        let retained_claim =
            match load_task_ingress_claim_by_request(&mut transaction, &expected_claim) {
                Ok(Some(claim)) => claim,
                Ok(None) => {
                    return rollback_load(
                        transaction,
                        error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt),
                    );
                }
                Err(load_error) => return rollback_load(transaction, load_error),
            };
        let stream_id_bytes = match digest_bytes(retained.submission.stream_id()) {
            Ok(bytes) => bytes,
            Err(load_error) => return rollback_load(transaction, load_error),
        };
        let loaded = match load_verified_stream(
            &mut transaction,
            retained.submission.identity(),
            &stream_id_bytes,
            &self.database_uuid,
            &self.store_receipt_persistence,
            &self.global_persistence,
            self.sql_profile,
        ) {
            Ok(loaded) => loaded,
            Err(load_error) => return rollback_load(transaction, load_error),
        };
        if !submission_matches_loaded_stream(&retained, &loaded.stream)
            || !ingress_claim_matches_loaded_stream(&retained_claim, &loaded.stream)
        {
            return rollback_load(
                transaction,
                error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt),
            );
        }
        transaction
            .commit()
            .map_err(|database| map_database_error(&database))?;
        Ok(Some(PostgresTaskSubmissionLoad {
            submission: retained.submission,
            ledger: PostgresTaskLedgerLoad {
                stream: loaded.stream,
                retained_checkpoint: loaded.retained_checkpoint,
                physical_head: loaded.physical_head,
                persistence: self.global_persistence.clone(),
                autonomy_state: loaded.autonomy_state,
            },
        }))
    }

    /// Loads the fixed foreman stream and verifies every child row against the
    /// authoritative Ledger replay in one repeatable-read transaction.
    ///
    /// # Errors
    ///
    /// Missing, extra, malformed, unknown-version, or cross-linked rows fail closed.
    pub fn load_foreman_records(
        &mut self,
    ) -> PostgresTaskLedgerResult<Vec<VerifiedForemanSnapshotRecord>> {
        self.load_foreman_replay().map(|replay| replay.records)
    }

    /// Loads one same-transaction foreman replay including the authoritative
    /// Ledger digests and all verified child records.
    ///
    /// # Errors
    ///
    /// Missing, extra, malformed, unknown-version, or cross-linked rows fail closed.
    pub fn load_foreman_replay(&mut self) -> PostgresTaskLedgerResult<PostgresForemanReplay> {
        self.ensure_reconcilable()?;
        if !self.sql_profile.supports_foreman() {
            return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt));
        }
        let identity =
            foreman_coordination_identity().map_err(|ledger| map_ledger_error(&ledger))?;
        let stream_id = VerifiedStream::vacant(identity.clone(), RuntimeKind::Live)
            .map_err(|ledger| map_ledger_error(&ledger))?
            .head()
            .stream_id()
            .clone();
        let stream_id_bytes = digest_bytes(&stream_id)?;
        let mut transaction = self
            .client
            .build_transaction()
            .isolation_level(IsolationLevel::RepeatableRead)
            .read_only(true)
            .start()
            .map_err(|database| map_database_error(&database))?;
        if let Err(database) = transaction.batch_execute(READ_TRANSACTION_SETTINGS) {
            return rollback_load(transaction, map_database_error(&database));
        }
        let loaded = match load_verified_stream(
            &mut transaction,
            &identity,
            &stream_id_bytes,
            &self.database_uuid,
            &self.store_receipt_persistence,
            &self.global_persistence,
            self.sql_profile,
        ) {
            Ok(loaded) => loaded,
            Err(load_error) => return rollback_load(transaction, load_error),
        };
        let records = match load_foreman_records(&mut transaction, &stream_id_bytes, &loaded.stream)
        {
            Ok(records) => records,
            Err(load_error) => return rollback_load(transaction, load_error),
        };
        transaction
            .commit()
            .map_err(|database| map_database_error(&database))?;
        Ok(PostgresForemanReplay {
            ledger: PostgresTaskLedgerLoad {
                stream: loaded.stream,
                retained_checkpoint: loaded.retained_checkpoint,
                physical_head: loaded.physical_head,
                persistence: self.global_persistence.clone(),
                autonomy_state: loaded.autonomy_state,
            },
            records,
        })
    }

    /// Executes one Task-Ledger-planned foreman snapshot under the exact
    /// current Writer Lease and persists its child row in the same transaction.
    ///
    /// # Errors
    ///
    /// Fails closed on stale authority, substituted plan, corrupt replay, or
    /// any transaction whose commit result is not known.
    pub fn execute_foreman(
        &mut self,
        foreman_plan: &ForemanSnapshotAppendPlan,
        expected_authority: &StoreAuthorityHead,
        writer_authority: &WriterLeaseAuthorityHead,
    ) -> PostgresTaskLedgerResult<PostgresTaskLedgerExecution> {
        if !self.sql_profile.supports_foreman() {
            return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt));
        }
        let command = foreman_plan
            .ledger_plan()
            .command_record()
            .request()
            .clone();
        self.execute_with_writer_authority(
            &command,
            expected_authority,
            Some(writer_authority),
            None,
            Some(foreman_plan),
            None,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn execute_with_writer_authority(
        &mut self,
        command: &AppendCommand,
        expected_authority: &StoreAuthorityHead,
        writer_authority: Option<&WriterLeaseAuthorityHead>,
        autonomy_plan: Option<&AutonomyReceiptAppendPlan>,
        foreman_plan: Option<&ForemanSnapshotAppendPlan>,
        submission: Option<&TaskSubmissionEnvelope>,
        ingress_claim: Option<&TaskIngressClaim>,
    ) -> PostgresTaskLedgerResult<PostgresTaskLedgerExecution> {
        self.ensure_reconcilable()?;
        if command.expected_head().runtime() != RuntimeKind::Live
            || (autonomy_plan.is_none()
                && command.kind() == lattice_task_ledger::LedgerEventKind::AutonomyReceiptRecorded)
            || (foreman_plan.is_none()
                && command.kind() == lattice_task_ledger::LedgerEventKind::ForemanSnapshotRecorded)
            || (autonomy_plan.is_some() && foreman_plan.is_some())
            || submission.is_some_and(|envelope| !submission_matches_command(envelope, command))
            || (submission.is_none() && command_is_general_submission(command))
            || submission.is_some() && (autonomy_plan.is_some() || foreman_plan.is_some())
            || ingress_claim.is_some_and(|claim| !ingress_claim_matches_command(claim, command))
            || submission.is_some_and(|envelope| {
                ingress_claim.is_none_or(|claim| {
                    claim.request_kind() != TaskIngressRequestKind::GeneralTask
                        || claim.ingress_id() != envelope.ingress_id()
                        || claim.client_request_id() != envelope.client_request_id()
                        || claim.stream_id() != envelope.stream_id()
                })
            })
            || ingress_claim.is_some_and(|claim| {
                claim.request_kind() == TaskIngressRequestKind::GeneralTask && submission.is_none()
            })
            || ingress_claim.is_some() && (autonomy_plan.is_some() || foreman_plan.is_some())
        {
            return Err(error(PostgresTaskLedgerErrorKind::Malformed));
        }
        let identity = command.expected_head().identity().clone();
        let canonical_stream = VerifiedStream::vacant(identity.clone(), RuntimeKind::Live)
            .map_err(|ledger| map_ledger_error(&ledger))?;
        if canonical_stream.head().stream_id() != command.expected_head().stream_id() {
            return Err(error(PostgresTaskLedgerErrorKind::Malformed));
        }
        if writer_authority
            .is_some_and(|authority| !writer_authority_matches_identity(authority, &identity))
        {
            return Err(error(PostgresTaskLedgerErrorKind::AuthorityMismatch));
        }
        let stream_id_bytes = digest_bytes(canonical_stream.head().stream_id())?;
        let command_id = command.command_id().as_str().to_owned();

        for retry_count in 0..=MAX_LIVE_SERIALIZATION_RETRIES {
            match run_execute_attempt(
                &mut self.client,
                &self.database_uuid,
                &self.store_receipt_persistence,
                &self.global_persistence,
                self.sql_profile,
                &identity,
                &stream_id_bytes,
                command.clone(),
                &command_id,
                expected_authority.clone(),
                writer_authority,
                autonomy_plan,
                foreman_plan,
                submission,
                ingress_claim,
            ) {
                Ok(execution) => return Ok(execution),
                Err(AttemptFailure::Retryable) if retry_count < MAX_LIVE_SERIALIZATION_RETRIES => {}
                Err(AttemptFailure::Retryable) => {
                    return Err(error(PostgresTaskLedgerErrorKind::SerializationExhausted));
                }
                Err(AttemptFailure::CommitOutcomeUnknown) => {
                    self.commit_outcome_unknown = true;
                    return Err(error(PostgresTaskLedgerErrorKind::CommitOutcomeUnknown));
                }
                Err(AttemptFailure::Terminal(execution_error)) => return Err(execution_error),
            }
        }
        Err(error(PostgresTaskLedgerErrorKind::TransactionFailed))
    }

    fn ensure_reconcilable(&self) -> PostgresTaskLedgerResult<()> {
        if self.commit_outcome_unknown {
            Err(error(PostgresTaskLedgerErrorKind::CommitOutcomeUnknown))
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TaskLedgerSqlProfile {
    V3,
    V5,
    V6,
    V7,
}

impl TaskLedgerSqlProfile {
    const fn ledger_prepare_sql(self) -> &'static str {
        match self {
            Self::V3 => LEDGER_PREPARE_V3_SQL,
            Self::V5 | Self::V6 | Self::V7 => LEDGER_PREPARE_V5_SQL,
        }
    }

    const fn ledger_head_sql(self) -> &'static str {
        match self {
            Self::V3 => LEDGER_HEAD_V3_SQL,
            Self::V5 | Self::V6 => LEDGER_HEAD_V5_SQL,
            Self::V7 => LEDGER_HEAD_V7_SQL,
        }
    }

    const fn ledger_events_sql(self) -> &'static str {
        match self {
            Self::V3 => LEDGER_EVENTS_V3_SQL,
            Self::V5 | Self::V6 | Self::V7 => LEDGER_EVENTS_V5_SQL,
        }
    }

    const fn ledger_commands_sql(self) -> &'static str {
        match self {
            Self::V3 => LEDGER_COMMANDS_V3_SQL,
            Self::V5 | Self::V6 | Self::V7 => LEDGER_COMMANDS_V5_SQL,
        }
    }

    const fn ledger_finalize_sql(self) -> &'static str {
        match self {
            Self::V3 => LEDGER_FINALIZE_V3_SQL,
            Self::V5 | Self::V6 | Self::V7 => LEDGER_FINALIZE_V5_SQL,
        }
    }

    const fn general_ledger_finalize_sql(self) -> Option<&'static str> {
        match self {
            Self::V7 => Some(LEDGER_FINALIZE_GENERAL_V7_SQL),
            Self::V3 | Self::V5 | Self::V6 => None,
        }
    }

    const fn store_prepare_sql(self) -> &'static str {
        match self {
            Self::V3 => STORE_PREPARE_V3_SQL,
            Self::V5 | Self::V6 | Self::V7 => STORE_PREPARE_V5_SQL,
        }
    }

    const fn store_finalize_sql(self) -> &'static str {
        match self {
            Self::V3 => STORE_FINALIZE_V3_SQL,
            Self::V5 | Self::V6 | Self::V7 => STORE_FINALIZE_V5_SQL,
        }
    }

    const fn store_current_sql(self) -> &'static str {
        match self {
            Self::V3 => STORE_CURRENT_V3_SQL,
            Self::V5 | Self::V6 | Self::V7 => STORE_CURRENT_V5_SQL,
        }
    }

    const fn supports_autonomy(self) -> bool {
        matches!(self, Self::V5 | Self::V6 | Self::V7)
    }

    const fn autonomy_receipts_sql(self) -> Option<&'static str> {
        match self {
            Self::V3 => None,
            Self::V5 | Self::V6 | Self::V7 => Some(LEDGER_AUTONOMY_RECEIPTS_SQL),
        }
    }

    const fn autonomy_record_sql(self) -> Option<&'static str> {
        match self {
            Self::V3 => None,
            Self::V5 | Self::V6 | Self::V7 => Some(LEDGER_RECORD_AUTONOMY_RECEIPT_SQL),
        }
    }

    const fn supports_foreman(self) -> bool {
        matches!(self, Self::V6 | Self::V7)
    }

    const fn supports_submission(self) -> bool {
        matches!(self, Self::V7)
    }

    const fn has_global_profile_parameters(self) -> bool {
        matches!(self, Self::V5 | Self::V6 | Self::V7)
    }
}

fn global_ledger_sql_profile(schema_version: u16) -> Option<TaskLedgerSqlProfile> {
    if schema_version == LEGACY_GLOBAL_LEDGER_SCHEMA_VERSION {
        Some(TaskLedgerSqlProfile::V3)
    } else if schema_version == CURRENT_GLOBAL_LEDGER_SCHEMA_VERSION {
        Some(TaskLedgerSqlProfile::V5)
    } else if schema_version == FOREMAN_GLOBAL_LEDGER_SCHEMA_VERSION {
        Some(TaskLedgerSqlProfile::V6)
    } else if schema_version == SUBMISSION_GLOBAL_LEDGER_SCHEMA_VERSION {
        Some(TaskLedgerSqlProfile::V7)
    } else {
        None
    }
}

struct LoadedStream {
    stream: VerifiedStream,
    retained_checkpoint: LedgerCheckpoint,
    physical_head: StorePhysicalHead,
    store_receipts: BTreeMap<String, StoreTransactionReceipt>,
    record_set_digests: BTreeMap<String, ContentDigest>,
    autonomy_state: VerifiedAutonomyReceiptState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RetainedTaskSubmission {
    submission: TaskSubmissionEnvelope,
    event_sequence: u64,
    event_digest: ContentDigest,
    command_id: String,
    request_digest: ContentDigest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RetainedTaskIngressClaim {
    claim: TaskIngressClaim,
    event_sequence: u64,
    event_digest: ContentDigest,
    command_id: String,
    command_request_digest: ContentDigest,
}

struct LedgerPrepareRow {
    stream_found: bool,
    command_found: bool,
    retained_request_digest: Option<ContentDigest>,
    retained_receipt_digest: Option<ContentDigest>,
    retained_base_checkpoint_digest: Option<ContentDigest>,
    retained_result_checkpoint_digest: Option<ContentDigest>,
    retained_store_transaction_id: Option<String>,
    terminal_found: bool,
    physical_state_digest: Option<ContentDigest>,
}

enum AttemptFailure {
    Retryable,
    CommitOutcomeUnknown,
    Terminal(PostgresTaskLedgerError),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CommitFailureClass {
    Retryable,
    OutcomeUnknown,
    Terminal,
}

#[allow(clippy::too_many_lines)]
fn load_verified_stream<C: GenericClient>(
    client: &mut C,
    expected_identity: &TaskLedgerStreamIdentity,
    stream_id_bytes: &[u8],
    database_uuid: &str,
    store_persistence: &StorePersistenceEvidence,
    global_persistence: &PostgresTaskLedgerPersistenceEvidence,
    sql_profile: TaskLedgerSqlProfile,
) -> PostgresTaskLedgerResult<LoadedStream> {
    let global_schema_version = i16::try_from(global_persistence.schema_version())
        .map_err(|_| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))?;
    let global_manifest_sha256 = global_persistence.manifest_digest().as_str().to_owned();
    let stream_id = bytes_digest(stream_id_bytes)?;
    let scope = ledger_scope(expected_identity, stream_id.clone())?;
    let vacant = VerifiedStream::vacant(expected_identity.clone(), RuntimeKind::Live)
        .map_err(|ledger| map_ledger_error(&ledger))?;
    let project_id = expected_identity.project_id().as_str();
    let project_snapshot_id = expected_identity.project_snapshot_id().as_str();
    let head_params = global_profile_params(
        sql_profile,
        &global_schema_version,
        &global_manifest_sha256,
        [
            &stream_id_bytes as &(dyn ToSql + Sync),
            &project_id,
            &project_snapshot_id,
        ],
    );
    let head_row = client
        .query_opt(sql_profile.ledger_head_sql(), &head_params)
        .map_err(|database| map_database_error(&database))?;

    let Some(head_row) = head_row else {
        let event_params = global_profile_params(
            sql_profile,
            &global_schema_version,
            &global_manifest_sha256,
            [&stream_id_bytes as &(dyn ToSql + Sync)],
        );
        let events = client
            .query(sql_profile.ledger_events_sql(), &event_params)
            .map_err(|database| map_database_error(&database))?;
        let command_params = global_profile_params(
            sql_profile,
            &global_schema_version,
            &global_manifest_sha256,
            [&stream_id_bytes as &(dyn ToSql + Sync)],
        );
        let commands = client
            .query(sql_profile.ledger_commands_sql(), &command_params)
            .map_err(|database| map_database_error(&database))?;
        let autonomy_rows_exist = if let Some(sql) = sql_profile.autonomy_receipts_sql() {
            !client
                .query(sql, &[&stream_id_bytes])
                .map_err(|database| map_database_error(&database))?
                .is_empty()
        } else {
            false
        };
        if !events.is_empty() || !commands.is_empty() || autonomy_rows_exist {
            return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt));
        }
        let physical = read_store_current_head(
            client,
            &scope,
            database_uuid,
            store_persistence,
            global_persistence,
            sql_profile,
        )?;
        let genesis = genesis_head(RuntimeKind::Live, scope)
            .map_err(|_| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))?;
        if physical != genesis {
            return Err(error(PostgresTaskLedgerErrorKind::PhysicalStateMismatch));
        }
        return Ok(LoadedStream {
            retained_checkpoint: vacant.checkpoint().clone(),
            stream: vacant,
            physical_head: physical,
            store_receipts: BTreeMap::new(),
            record_set_digests: BTreeMap::new(),
            autonomy_state: VerifiedAutonomyReceiptState::NotApplicable,
        });
    };

    let subject_offset = if sql_profile == TaskLedgerSqlProfile::V7 {
        2
    } else {
        0
    };
    if head_row.len() != 37 + subject_offset {
        return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt));
    }
    let retained_stream_id = row_digest(&head_row, 0)?;
    let ledger_schema_version: String = row_value(&head_row, 1)?;
    let head_contract_version: i16 = row_value(&head_row, 2)?;
    let producer_id: String = row_value(&head_row, 3)?;
    let producer_version: String = row_value(&head_row, 4)?;
    let runtime: String = row_value(&head_row, 5)?;
    if retained_stream_id != stream_id
        || ledger_schema_version != LEDGER_SCHEMA_VERSION
        || head_contract_version != i16::try_from(CONTRACT_VERSION).unwrap_or_default()
        || producer_id != TASK_LEDGER_PRODUCER_ID
        || producer_version != TASK_LEDGER_PRODUCER_VERSION
        || runtime != "LIVE"
    {
        return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt));
    }
    let project_id = ProjectId::new(row_value::<String>(&head_row, 6)?)
        .map_err(|_| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))?;
    let project_snapshot_id = ProjectSnapshotId::new(row_value::<String>(&head_row, 7)?)
        .map_err(|_| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))?;
    let task_id = TaskId::new(row_value::<String>(&head_row, 8)?)
        .map_err(|_| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))?;
    let task_revision = row_value::<String>(&head_row, 9)?;
    let identity = if sql_profile == TaskLedgerSqlProfile::V7 {
        let subject_kind: String = row_value(&head_row, 10)?;
        let subject_digest = row_digest(&head_row, 11)?;
        let task_spec_digest = row_optional_digest(&head_row, 12)?;
        let accounting_currency: Option<String> = row_value(&head_row, 13)?;
        match subject_kind.as_str() {
            "TASK_SPEC"
                if task_spec_digest.as_ref() == Some(&subject_digest)
                    && accounting_currency.is_some() =>
            {
                TaskLedgerStreamIdentity::new(
                    project_id,
                    project_snapshot_id,
                    task_id,
                    task_revision,
                    task_spec_digest
                        .ok_or_else(|| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))?,
                    accounting_currency
                        .ok_or_else(|| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))?,
                )
            }
            "GENERAL_TASK_INTAKE"
                if task_spec_digest.is_none() && accounting_currency.is_none() =>
            {
                TaskLedgerStreamIdentity::new_general_task_intake(
                    project_id,
                    project_snapshot_id,
                    task_id,
                    task_revision,
                    subject_digest,
                )
            }
            _ => return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt)),
        }
    } else {
        TaskLedgerStreamIdentity::new(
            project_id,
            project_snapshot_id,
            task_id,
            task_revision,
            row_digest(&head_row, 10)?,
            row_value::<String>(&head_row, 11)?,
        )
    }
    .map_err(|_| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))?;
    if &identity != expected_identity {
        return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt));
    }
    let claimed_head = task_head(
        identity.clone(),
        stream_id.clone(),
        parse_u64_text(&row_value::<String>(&head_row, 12 + subject_offset)?)?,
        row_digest(&head_row, 13 + subject_offset)?,
        parse_u64_text(&row_value::<String>(&head_row, 14 + subject_offset)?)?,
        row_digest(&head_row, 15 + subject_offset)?,
        row_digest(&head_row, 16 + subject_offset)?,
    )?;
    let counters = resource_counters(
        &row_value::<String>(&head_row, 17 + subject_offset)?,
        &row_value::<String>(&head_row, 18 + subject_offset)?,
        &row_value::<String>(&head_row, 19 + subject_offset)?,
        &row_value::<String>(&head_row, 20 + subject_offset)?,
        &row_value::<String>(&head_row, 21 + subject_offset)?,
        row_value::<String>(&head_row, 22 + subject_offset)?,
    )?;
    let retained_counts = [
        parse_u64_text(&row_value::<String>(&head_row, 23 + subject_offset)?)?,
        parse_u64_text(&row_value::<String>(&head_row, 24 + subject_offset)?)?,
        parse_u64_text(&row_value::<String>(&head_row, 25 + subject_offset)?)?,
    ];
    let checkpoint_schema: String = row_value(&head_row, 26 + subject_offset)?;
    if checkpoint_schema != LEDGER_CHECKPOINT_SCHEMA_VERSION {
        return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt));
    }
    let retained_checkpoint = LedgerCheckpoint::from_retained(
        stream_id.clone(),
        RuntimeKind::Live,
        row_digest(&head_row, 27 + subject_offset)?,
    );
    let actual_counts = [
        parse_u64_text(&row_value::<String>(&head_row, 28 + subject_offset)?)?,
        parse_u64_text(&row_value::<String>(&head_row, 29 + subject_offset)?)?,
        parse_u64_text(&row_value::<String>(&head_row, 30 + subject_offset)?)?,
    ];
    if retained_counts != actual_counts {
        return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt));
    }
    let physical_found: bool = row_value(&head_row, 31 + subject_offset)?;
    let physical_revision: Option<i64> = row_value(&head_row, 32 + subject_offset)?;
    let physical_state: Option<Vec<u8>> = row_value(&head_row, 33 + subject_offset)?;
    let physical_head_digest: Option<Vec<u8>> = row_value(&head_row, 34 + subject_offset)?;
    let global_schema_version: i16 = row_value(&head_row, 35 + subject_offset)?;
    let global_manifest_sha256: String = row_value(&head_row, 36 + subject_offset)?;
    if !global_persistence_matches(
        global_schema_version,
        &global_manifest_sha256,
        global_persistence,
    ) {
        return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt));
    }
    let physical_head = match (
        physical_found,
        physical_revision,
        physical_state.as_deref(),
        physical_head_digest.as_deref(),
    ) {
        (true, Some(revision), Some(state), Some(head)) => {
            stored_physical_head(&scope, revision, state, head)?
        }
        _ => return Err(error(PostgresTaskLedgerErrorKind::PhysicalStateMismatch)),
    };
    validate_physical_command_count(&physical_head, retained_counts[1], actual_counts[1])?;
    if physical_head.state_digest() != retained_checkpoint.checkpoint_digest() {
        return Err(error(PostgresTaskLedgerErrorKind::PhysicalStateMismatch));
    }

    let event_params = global_profile_params(
        sql_profile,
        &global_schema_version,
        &global_manifest_sha256,
        [&stream_id_bytes as &(dyn ToSql + Sync)],
    );
    let event_rows = client
        .query(sql_profile.ledger_events_sql(), &event_params)
        .map_err(|database| map_database_error(&database))?;
    let mut events = Vec::with_capacity(event_rows.len());
    let mut outboxes = Vec::new();
    for row in &event_rows {
        let (event, outbox) = parse_event_row(row, &identity, &stream_id)?;
        events.push(event);
        if let Some(outbox) = outbox {
            outboxes.push(outbox);
        }
    }
    let command_params = global_profile_params(
        sql_profile,
        &global_schema_version,
        &global_manifest_sha256,
        [&stream_id_bytes as &(dyn ToSql + Sync)],
    );
    let command_rows = client
        .query(sql_profile.ledger_commands_sql(), &command_params)
        .map_err(|database| map_database_error(&database))?;
    let mut commands = Vec::with_capacity(command_rows.len());
    let mut store_receipts = BTreeMap::new();
    let mut record_set_digests = BTreeMap::new();
    for row in &command_rows {
        let parsed =
            parse_command_row(row, &identity, &stream_id, database_uuid, store_persistence)?;
        if store_receipts
            .insert(parsed.command_id.clone(), parsed.store_receipt)
            .is_some()
            || record_set_digests
                .insert(parsed.command_id, parsed.record_set_digest)
                .is_some()
        {
            return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt));
        }
        commands.push(parsed.record);
    }
    if usize::try_from(retained_counts[0]).ok() != Some(events.len())
        || usize::try_from(retained_counts[1]).ok() != Some(commands.len())
        || usize::try_from(retained_counts[2]).ok() != Some(outboxes.len())
    {
        return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt));
    }
    let snapshot = UntrustedLedgerSnapshot {
        identity,
        claimed_head,
        events,
        commands,
        outboxes,
        claimed_counters: counters,
    };
    let stream = verify_untrusted_snapshot_against_checkpoint(&snapshot, &retained_checkpoint)
        .map_err(|ledger| map_ledger_error(&ledger))?;
    validate_store_bindings(&stream, &store_receipts, &record_set_digests)?;
    let autonomy_state =
        load_autonomy_receipt_for_profile(client, stream_id_bytes, &stream, sql_profile)?;
    Ok(LoadedStream {
        stream,
        retained_checkpoint,
        physical_head,
        store_receipts,
        record_set_digests,
        autonomy_state,
    })
}

struct ParsedCommandRow {
    command_id: String,
    record: UntrustedCommandRecord,
    record_set_digest: ContentDigest,
    store_receipt: StoreTransactionReceipt,
}

fn row_value<T: FromSqlOwned>(row: &Row, index: usize) -> PostgresTaskLedgerResult<T> {
    row.try_get(index)
        .map_err(|_| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))
}

fn global_persistence_matches(
    observed_schema_version: i16,
    observed_manifest_sha256: &str,
    expected: &PostgresTaskLedgerPersistenceEvidence,
) -> bool {
    i16::try_from(expected.schema_version()).ok() == Some(observed_schema_version)
        && observed_manifest_sha256 == expected.manifest_digest().as_str()
}

fn row_digest(row: &Row, index: usize) -> PostgresTaskLedgerResult<ContentDigest> {
    bytes_digest(&row_value::<Vec<u8>>(row, index)?)
}

fn row_optional_digest(row: &Row, index: usize) -> PostgresTaskLedgerResult<Option<ContentDigest>> {
    row_value::<Option<Vec<u8>>>(row, index)?
        .as_deref()
        .map(bytes_digest)
        .transpose()
}

fn ingress_claim_event_action_matches(
    request_kind: TaskIngressRequestKind,
    event_action: Option<&str>,
) -> bool {
    match request_kind {
        TaskIngressRequestKind::ControlledCodexCanary => matches!(
            event_action,
            Some("CONTROLLED_CODEX_CANARY" | "CONTROLLED_CODEX_CANARY_AUTONOMY_V1")
        ),
        TaskIngressRequestKind::GeneralTask => {
            event_action == Some(TaskCreatedProfile::GeneralTaskIntakeV1.action())
        }
    }
}

fn ingress_claim_command_matches(claim: &TaskIngressClaim, command_id: &str) -> bool {
    command_id
        .strip_prefix("mcp-submit:")
        .is_some_and(|client_request_id| client_request_id == claim.client_request_id())
}

fn parse_task_ingress_claim_row(
    row: &Row,
    offset: usize,
    expected: Option<&TaskIngressClaim>,
) -> PostgresTaskLedgerResult<RetainedTaskIngressClaim> {
    if row.len() != offset + 13 {
        return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt));
    }
    let raw = UntrustedTaskIngressClaim {
        schema_version: row_value(row, offset)?,
        ingress_id: row_value(row, offset + 1)?,
        client_request_id: row_value(row, offset + 2)?,
        request_kind: row_value(row, offset + 3)?,
        request_digest: row_digest(row, offset + 4)?,
        stream_id: row_digest(row, offset + 5)?,
    };
    let claim = match expected {
        Some(expected) => verify_untrusted_task_ingress_claim(&raw, expected),
        None => verify_untrusted_task_ingress_claim_structure(&raw),
    }
    .map_err(|ledger| map_ledger_error(&ledger))?;
    let event_sequence = parse_u64_text(&row_value::<String>(row, offset + 6)?)?;
    let command_id: String = row_value(row, offset + 8)?;
    let event_kind: Option<String> = row_value(row, offset + 10)?;
    let event_action: Option<String> = row_value(row, offset + 11)?;
    let event_outcome: Option<String> = row_value(row, offset + 12)?;
    if event_sequence != 1
        || CommandId::new(command_id.clone()).is_err()
        || !ingress_claim_command_matches(&claim, &command_id)
        || event_kind.as_deref() != Some("TASK_CREATED")
        || !ingress_claim_event_action_matches(claim.request_kind(), event_action.as_deref())
        || event_outcome.as_deref() != Some("RECORDED")
    {
        return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt));
    }
    Ok(RetainedTaskIngressClaim {
        claim,
        event_sequence,
        event_digest: row_digest(row, offset + 7)?,
        command_id,
        command_request_digest: row_digest(row, offset + 9)?,
    })
}

fn prepare_task_ingress_claim<C: GenericClient>(
    client: &mut C,
    claim: &TaskIngressClaim,
) -> PostgresTaskLedgerResult<Option<RetainedTaskIngressClaim>> {
    let ingress_request_digest = digest_bytes(claim.request_digest())?;
    let stream_id = digest_bytes(claim.stream_id())?;
    let row = client
        .query_one(
            TASK_INGRESS_PREPARE_SQL,
            &[
                &claim.ingress_id(),
                &claim.client_request_id(),
                &claim.request_kind().as_str(),
                &ingress_request_digest,
                &stream_id,
            ],
        )
        .map_err(|database| map_database_error(&database))?;
    if row.len() != 14 {
        return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt));
    }
    if !row_value::<bool>(&row, 0)? {
        return Ok(None);
    }
    parse_task_ingress_claim_row(&row, 1, Some(claim)).map(Some)
}

fn load_task_ingress_claim_by_request<C: GenericClient>(
    client: &mut C,
    expected: &TaskIngressClaim,
) -> PostgresTaskLedgerResult<Option<RetainedTaskIngressClaim>> {
    client
        .query_opt(
            TASK_INGRESS_READ_BY_REQUEST_SQL,
            &[&expected.ingress_id(), &expected.client_request_id()],
        )
        .map_err(|database| map_database_error(&database))?
        .as_ref()
        .map(|row| parse_task_ingress_claim_row(row, 0, Some(expected)))
        .transpose()
}

fn record_task_ingress_claim(
    client: &mut Transaction<'_>,
    claim: &TaskIngressClaim,
    event: &lattice_task_ledger::LedgerEvent,
) -> Result<(), AttemptFailure> {
    let ingress_request_digest =
        digest_bytes(claim.request_digest()).map_err(AttemptFailure::Terminal)?;
    let stream_id = digest_bytes(claim.stream_id()).map_err(AttemptFailure::Terminal)?;
    let event_digest = digest_bytes(event.event_digest()).map_err(AttemptFailure::Terminal)?;
    let command_request_digest =
        digest_bytes(event.request_digest()).map_err(AttemptFailure::Terminal)?;
    let event_sequence = event.sequence().to_string();
    let params: [&(dyn ToSql + Sync); 10] = [
        &claim.schema_version(),
        &claim.ingress_id(),
        &claim.client_request_id(),
        &claim.request_kind().as_str(),
        &ingress_request_digest,
        &stream_id,
        &event_sequence,
        &event_digest,
        &event.command_id().as_str(),
        &command_request_digest,
    ];
    let row = client
        .query_one(TASK_INGRESS_RECORD_SQL, &params)
        .map_err(|database| classify_query_error(&database))?;
    if row.len() != 1
        || row_value::<String>(&row, 0).map_err(AttemptFailure::Terminal)? != "RECORDED"
    {
        return Err(AttemptFailure::Terminal(error(
            PostgresTaskLedgerErrorKind::RetainedRowCorrupt,
        )));
    }
    Ok(())
}

fn parse_submission_row(
    row: &Row,
    offset: usize,
) -> PostgresTaskLedgerResult<RetainedTaskSubmission> {
    if row.len() != offset + 21 {
        return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt));
    }
    let task_subject_kind: String = row_value(row, offset + 10)?;
    if task_subject_kind != TaskLedgerSubjectKind::GeneralTaskIntake.as_str() {
        return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt));
    }
    let identity = TaskLedgerStreamIdentity::new_general_task_intake(
        ProjectId::new(row_value::<String>(row, offset + 6)?)
            .map_err(|_| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))?,
        ProjectSnapshotId::new(row_value::<String>(row, offset + 7)?)
            .map_err(|_| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))?,
        TaskId::new(row_value::<String>(row, offset + 8)?)
            .map_err(|_| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))?,
        row_value::<String>(row, offset + 9)?,
        row_digest(row, offset + 11)?,
    )
    .map_err(|_| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))?;
    let raw = UntrustedTaskSubmissionEnvelope {
        schema_version: row_value(row, offset)?,
        ingress_id: row_value(row, offset + 1)?,
        client_request_id: row_value(row, offset + 2)?,
        objective: row_value(row, offset + 3)?,
        project_display_name: row_value(row, offset + 4)?,
        project_authority_receipt_digest: row_digest(row, offset + 5)?,
        identity,
        stream_id: row_digest(row, offset + 12)?,
        task_ref: digest(&row_value::<String>(row, offset + 13)?)?,
        admission_action: row_value(row, offset + 14)?,
        envelope_digest: row_digest(row, offset + 15)?,
    };
    let submission =
        verify_untrusted_task_submission(&raw).map_err(|ledger| map_ledger_error(&ledger))?;
    let expected_claim = TaskIngressClaim::general_submission(&submission)
        .map_err(|ledger| map_ledger_error(&ledger))?;
    if &row_digest(row, offset + 20)? != expected_claim.request_digest() {
        return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt));
    }
    Ok(RetainedTaskSubmission {
        submission,
        event_sequence: parse_u64_text(&row_value::<String>(row, offset + 16)?)?,
        event_digest: row_digest(row, offset + 17)?,
        command_id: row_value(row, offset + 18)?,
        request_digest: row_digest(row, offset + 19)?,
    })
}

fn prepare_task_submission<C: GenericClient>(
    client: &mut C,
    submission: &TaskSubmissionEnvelope,
) -> PostgresTaskLedgerResult<Option<RetainedTaskSubmission>> {
    let envelope_digest = digest_bytes(submission.envelope_digest())?;
    let row = client
        .query_one(
            TASK_SUBMISSION_PREPARE_SQL,
            &[
                &submission.ingress_id(),
                &submission.client_request_id(),
                &envelope_digest,
            ],
        )
        .map_err(|database| map_database_error(&database))?;
    if row.len() != 22 {
        return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt));
    }
    let found: bool = row_value(&row, 0)?;
    if !found {
        return Ok(None);
    }
    let retained = parse_submission_row(&row, 1)?;
    if retained.submission != *submission {
        return Err(error(PostgresTaskLedgerErrorKind::CommandSubstitution));
    }
    Ok(Some(retained))
}

fn load_task_submission_by_ref<C: GenericClient>(
    client: &mut C,
    task_ref: &ContentDigest,
) -> PostgresTaskLedgerResult<Option<RetainedTaskSubmission>> {
    let task_ref = task_ref.as_str();
    client
        .query_opt(TASK_SUBMISSION_READ_BY_REF_SQL, &[&task_ref])
        .map_err(|database| map_database_error(&database))?
        .as_ref()
        .map(|row| parse_submission_row(row, 0))
        .transpose()
}

fn record_task_submission<C: GenericClient>(
    client: &mut C,
    submission: &TaskSubmissionEnvelope,
    event: &lattice_task_ledger::LedgerEvent,
) -> PostgresTaskLedgerResult<()> {
    let authority_digest = digest_bytes(submission.project_authority_receipt_digest())?;
    let intake_digest = submission
        .identity()
        .general_task_intake_digest()
        .ok_or_else(|| error(PostgresTaskLedgerErrorKind::Malformed))?;
    let intake_digest = digest_bytes(intake_digest)?;
    let stream_id = digest_bytes(submission.stream_id())?;
    let envelope_digest = digest_bytes(submission.envelope_digest())?;
    let event_digest = digest_bytes(event.event_digest())?;
    let request_digest = digest_bytes(event.request_digest())?;
    let ingress_request_digest = TaskIngressClaim::general_submission(submission)
        .map_err(|ledger| map_ledger_error(&ledger))?;
    let ingress_request_digest = digest_bytes(ingress_request_digest.request_digest())?;
    let event_sequence = event.sequence().to_string();
    let params: [&(dyn ToSql + Sync); 21] = [
        &submission.schema_version(),
        &submission.ingress_id(),
        &submission.client_request_id(),
        &submission.objective(),
        &submission.project_display_name(),
        &authority_digest,
        &submission.identity().project_id().as_str(),
        &submission.identity().project_snapshot_id().as_str(),
        &submission.identity().task_id().as_str(),
        &submission.identity().task_revision(),
        &submission.identity().subject_kind().as_str(),
        &intake_digest,
        &stream_id,
        &submission.task_ref().as_str(),
        &submission.admission_action(),
        &envelope_digest,
        &event_sequence,
        &event_digest,
        &event.command_id().as_str(),
        &request_digest,
        &ingress_request_digest,
    ];
    let row = client
        .query_one(TASK_SUBMISSION_RECORD_SQL, &params)
        .map_err(|database| map_database_error(&database))?;
    if row.len() != 1 || row_value::<String>(&row, 0)? != "RECORDED" {
        return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt));
    }
    Ok(())
}

fn ingress_claim_matches_command(claim: &TaskIngressClaim, command: &AppendCommand) -> bool {
    command.kind() == LedgerEventKind::TaskCreated
        && command.expected_head().sequence() == 0
        && command.expected_head().stream_id() == claim.stream_id()
        && command.outcome() == lattice_task_ledger::LedgerOutcome::Recorded
        && match claim.request_kind() {
            TaskIngressRequestKind::ControlledCodexCanary => {
                command.action().as_str() == TaskCreatedProfile::AutonomyReceiptRequiredV1.action()
            }
            TaskIngressRequestKind::GeneralTask => command_is_general_submission(command),
        }
}

fn ingress_claim_matches_loaded_stream(
    retained: &RetainedTaskIngressClaim,
    stream: &VerifiedStream,
) -> bool {
    let Some(event) = stream
        .events()
        .iter()
        .find(|event| event.sequence() == retained.event_sequence)
    else {
        return false;
    };
    let Some(command) = stream
        .commands()
        .iter()
        .find(|record| record.request().command_id().as_str() == retained.command_id)
    else {
        return false;
    };
    let action_matches = match retained.claim.request_kind() {
        TaskIngressRequestKind::ControlledCodexCanary => matches!(
            event.action().as_str(),
            "CONTROLLED_CODEX_CANARY" | "CONTROLLED_CODEX_CANARY_AUTONOMY_V1"
        ),
        TaskIngressRequestKind::GeneralTask => {
            event.action().as_str() == TaskCreatedProfile::GeneralTaskIntakeV1.action()
        }
    };
    stream.head().stream_id() == retained.claim.stream_id()
        && event.stream_id() == retained.claim.stream_id()
        && event.kind() == LedgerEventKind::TaskCreated
        && action_matches
        && event.event_digest() == &retained.event_digest
        && event.command_id().as_str() == retained.command_id
        && event.request_digest() == &retained.command_request_digest
        && command.receipt().request_digest() == &retained.command_request_digest
        && command.receipt().event_digest() == Some(&retained.event_digest)
}

fn command_is_general_submission(command: &AppendCommand) -> bool {
    command.kind() == LedgerEventKind::TaskCreated
        && command.action().as_str() == TaskCreatedProfile::GeneralTaskIntakeV1.action()
}

fn submission_matches_command(
    submission: &TaskSubmissionEnvelope,
    command: &AppendCommand,
) -> bool {
    command_is_general_submission(command)
        && command.expected_head().sequence() == 0
        && command.expected_head().identity() == submission.identity()
        && command.expected_head().stream_id() == submission.stream_id()
        && command.subject_digest() == submission.envelope_digest()
        && command.reason_code().as_str() == "GENERAL_TASK_INTAKE_RECORDED"
        && command.diagnostic().is_none()
}

fn submission_matches_loaded_stream(
    retained: &RetainedTaskSubmission,
    stream: &VerifiedStream,
) -> bool {
    let Some(event) = stream.events().first() else {
        return false;
    };
    stream.identity() == retained.submission.identity()
        && event.stream_id() == retained.submission.stream_id()
        && event.sequence() == retained.event_sequence
        && event.event_digest() == &retained.event_digest
        && event.command_id().as_str() == retained.command_id
        && event.request_digest() == &retained.request_digest
        && event.kind() == LedgerEventKind::TaskCreated
        && event.action().as_str() == retained.submission.admission_action()
        && event.subject_digest() == retained.submission.envelope_digest()
        && event.diagnostic().is_none()
}

fn valid_submission_lookup_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 64
        && bytes.first().is_some_and(u8::is_ascii_alphanumeric)
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(*byte, b'.' | b'_' | b':' | b'-'))
}

fn is_zero_content_digest(value: &ContentDigest) -> bool {
    value.as_str().bytes().all(|byte| byte == b'0')
}

fn digest_bytes(value: &ContentDigest) -> PostgresTaskLedgerResult<Vec<u8>> {
    let bytes = value.as_str().as_bytes();
    if bytes.len() != 64 {
        return Err(error(PostgresTaskLedgerErrorKind::Malformed));
    }
    let mut output = Vec::with_capacity(32);
    for pair in bytes.chunks_exact(2) {
        let high =
            hex_nibble(pair[0]).ok_or_else(|| error(PostgresTaskLedgerErrorKind::Malformed))?;
        let low =
            hex_nibble(pair[1]).ok_or_else(|| error(PostgresTaskLedgerErrorKind::Malformed))?;
        output.push((high << 4) | low);
    }
    Ok(output)
}

fn bytes_digest(bytes: &[u8]) -> PostgresTaskLedgerResult<ContentDigest> {
    if bytes.len() != 32 {
        return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt));
    }
    let mut output = String::with_capacity(64);
    for byte in bytes {
        output.push(char::from(LOWER_HEX[usize::from(byte >> 4)]));
        output.push(char::from(LOWER_HEX[usize::from(byte & 0x0f)]));
    }
    digest(&output)
}

const fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        _ => None,
    }
}

fn map_database_error(database: &PostgresError) -> PostgresTaskLedgerError {
    let Some(db_error) = database.as_db_error() else {
        return error(PostgresTaskLedgerErrorKind::Unavailable);
    };
    error(database_error_kind(db_error.code().code()))
}

fn database_error_kind(code: &str) -> PostgresTaskLedgerErrorKind {
    match code {
        "LTX01" => PostgresTaskLedgerErrorKind::CommandSubstitution,
        "LPG01" => PostgresTaskLedgerErrorKind::ProjectRegistryCurrentnessConflict,
        "LPG02" => PostgresTaskLedgerErrorKind::ProjectRegistryInactive,
        "LAD01" => PostgresTaskLedgerErrorKind::AdmissionDenied,
        "LAU01" => PostgresTaskLedgerErrorKind::AuthorityMismatch,
        "LFW01" => PostgresTaskLedgerErrorKind::Malformed,
        "LRV01" => PostgresTaskLedgerErrorKind::RevisionOverflow,
        "LCP01" => PostgresTaskLedgerErrorKind::CheckpointCorrupt,
        "LCR01" | "LST01" | "LST02" => PostgresTaskLedgerErrorKind::RetainedRowCorrupt,
        "42501" | "55P03" | "57014" => PostgresTaskLedgerErrorKind::Unavailable,
        _ => PostgresTaskLedgerErrorKind::TransactionFailed,
    }
}

fn rollback_load<T>(
    transaction: Transaction<'_>,
    load_error: PostgresTaskLedgerError,
) -> PostgresTaskLedgerResult<T> {
    transaction
        .rollback()
        .map_err(|_| error(PostgresTaskLedgerErrorKind::TransactionFailed))?;
    Err(load_error)
}

fn ledger_scope(
    identity: &TaskLedgerStreamIdentity,
    stream_id: ContentDigest,
) -> PostgresTaskLedgerResult<StoreScope> {
    StoreScope::new(
        identity.project_id().clone(),
        identity.project_snapshot_id().clone(),
        StoreRepositoryOwner::TaskLedger,
        stream_id,
    )
    .map_err(|_| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))
}

fn task_head(
    identity: TaskLedgerStreamIdentity,
    stream_id: ContentDigest,
    sequence: u64,
    last_event_digest: ContentDigest,
    resource_revision: u64,
    resource_projection_digest: ContentDigest,
    head_digest: ContentDigest,
) -> PostgresTaskLedgerResult<TaskLedgerStreamHead> {
    TaskLedgerStreamHead::new(
        CONTRACT_VERSION,
        TASK_LEDGER_PRODUCER_ID,
        TASK_LEDGER_PRODUCER_VERSION,
        RuntimeKind::Live,
        identity,
        stream_id,
        sequence,
        last_event_digest,
        resource_revision,
        resource_projection_digest,
        head_digest,
    )
    .map_err(|_| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))
}

fn resource_counters(
    active_agents: &str,
    active_implementers: &str,
    elapsed_seconds: &str,
    attempt_number: &str,
    used_model_calls: &str,
    used_external_cost: String,
) -> PostgresTaskLedgerResult<ResourceCounters> {
    ResourceCounters::new(
        parse_u64_text(active_agents)?,
        parse_u64_text(active_implementers)?,
        parse_u64_text(elapsed_seconds)?,
        parse_u64_text(attempt_number)?,
        parse_u64_text(used_model_calls)?,
        used_external_cost,
    )
    .map_err(|_| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))
}

fn stored_physical_head(
    scope: &StoreScope,
    revision: i64,
    state_digest: &[u8],
    retained_head_digest: &[u8],
) -> PostgresTaskLedgerResult<StorePhysicalHead> {
    let revision = u64::try_from(revision)
        .ok()
        .and_then(|value| StoreRevision::new(value).ok())
        .ok_or_else(|| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))?;
    let canonical = physical_head(
        RuntimeKind::Live,
        scope.clone(),
        revision,
        bytes_digest(state_digest)?,
    )
    .map_err(|_| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))?;
    validate_physical_head(&canonical)
        .map_err(|_| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))?;
    if canonical.head_digest() != &bytes_digest(retained_head_digest)? {
        return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt));
    }
    Ok(canonical)
}

fn validate_physical_command_count(
    physical_head: &StorePhysicalHead,
    retained_command_count: u64,
    actual_command_count: u64,
) -> PostgresTaskLedgerResult<()> {
    let physical_revision = physical_head.revision().get();
    if physical_revision != retained_command_count || physical_revision != actual_command_count {
        return Err(error(PostgresTaskLedgerErrorKind::PhysicalStateMismatch));
    }
    Ok(())
}

fn read_store_current_head<C: GenericClient>(
    client: &mut C,
    scope: &StoreScope,
    database_uuid: &str,
    persistence: &StorePersistenceEvidence,
    global_persistence: &PostgresTaskLedgerPersistenceEvidence,
    sql_profile: TaskLedgerSqlProfile,
) -> PostgresTaskLedgerResult<StorePhysicalHead> {
    let aggregate = digest_bytes(scope.aggregate_key_digest())?;
    let global_schema_version = i16::try_from(global_persistence.schema_version())
        .map_err(|_| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))?;
    let global_manifest_sha256 = global_persistence.manifest_digest().as_str().to_owned();
    let project_id = scope.project_id().as_str();
    let project_snapshot_id = scope.project_snapshot_id().as_str();
    let repository_owner = scope.owner().as_str();
    let params = global_profile_params(
        sql_profile,
        &global_schema_version,
        &global_manifest_sha256,
        [
            &project_id as &(dyn ToSql + Sync),
            &project_snapshot_id,
            &repository_owner,
            &aggregate,
        ],
    );
    let row = client
        .query_one(sql_profile.store_current_sql(), &params)
        .map_err(|database| map_database_error(&database))?;
    if row.len() != 9 {
        return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt));
    }
    let retained_uuid: String = row_value(&row, 0)?;
    let retained_schema: i16 = row_value(&row, 1)?;
    let retained_manifest: String = row_value(&row, 2)?;
    let global_schema_version: i16 = row_value(&row, 7)?;
    let global_manifest_sha256: String = row_value(&row, 8)?;
    if retained_uuid != database_uuid
        || retained_schema != i16::try_from(persistence.schema_version()).unwrap_or_default()
        || retained_manifest != persistence.manifest_digest().as_str()
        || !global_persistence_matches(
            global_schema_version,
            &global_manifest_sha256,
            global_persistence,
        )
    {
        return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt));
    }
    let found: bool = row_value(&row, 3)?;
    let revision: Option<i64> = row_value(&row, 4)?;
    let state: Option<Vec<u8>> = row_value(&row, 5)?;
    let head: Option<Vec<u8>> = row_value(&row, 6)?;
    match (found, revision, state.as_deref(), head.as_deref()) {
        (false, None, None, None) => genesis_head(RuntimeKind::Live, scope.clone())
            .map_err(|_| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt)),
        (true, Some(revision), Some(state), Some(head)) => {
            stored_physical_head(scope, revision, state, head)
        }
        _ => Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt)),
    }
}

fn parse_event_row(
    row: &Row,
    identity: &TaskLedgerStreamIdentity,
    stream_id: &ContentDigest,
) -> PostgresTaskLedgerResult<(UntrustedLedgerEvent, Option<UntrustedOutboxAdmission>)> {
    if row.len() != 31 || row_digest(row, 0)? != *stream_id {
        return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt));
    }
    let sequence = parse_u64_text(&row_value::<String>(row, 1)?)?;
    let command_id: String = row_value(row, 4)?;
    let request_digest = row_digest(row, 5)?;
    let occurred_at: String = row_value(row, 7)?;
    let subject_digest = row_digest(row, 13)?;
    let diagnostic = optional_diagnostic(row, 14)?;
    let resource_snapshot = resource_snapshot_from_row(row, 15, 16)?;
    let event_digest = row_digest(row, 24)?;
    let event = UntrustedLedgerEvent {
        schema_version: row_value(row, 2)?,
        stream_identity: identity.clone(),
        stream_id: stream_id.clone(),
        sequence,
        previous_event_digest: row_digest(row, 3)?,
        command_id: command_id.clone(),
        request_digest: request_digest.clone(),
        correlation_id: row_value(row, 6)?,
        occurred_at: occurred_at.clone(),
        kind: row_value(row, 8)?,
        actor_id: row_value(row, 9)?,
        action: row_value(row, 10)?,
        outcome: row_value(row, 11)?,
        reason_code: row_value(row, 12)?,
        subject_digest,
        diagnostic,
        resource_snapshot,
        resource_revision: parse_u64_text(&row_value::<String>(row, 22)?)?,
        resource_projection_digest: row_digest(row, 23)?,
        event_digest: event_digest.clone(),
    };
    let outbox_found: bool = row_value(row, 25)?;
    let admission_digest: Option<Vec<u8>> = row_value(row, 26)?;
    let admission_schema: Option<String> = row_value(row, 27)?;
    let admission_state: Option<String> = row_value(row, 28)?;
    let intent_digest: Option<Vec<u8>> = row_value(row, 29)?;
    let admission_occurred_at: Option<String> = row_value(row, 30)?;
    let outbox = match (
        outbox_found,
        admission_digest.as_deref(),
        admission_schema,
        admission_state,
        intent_digest.as_deref(),
        admission_occurred_at,
    ) {
        (false, None, None, None, None, None) => None,
        (true, Some(admission), Some(schema), Some(state), Some(intent), Some(outbox_time)) => {
            Some(UntrustedOutboxAdmission {
                schema_version: schema,
                stream_identity: identity.clone(),
                stream_id: stream_id.clone(),
                event_sequence: sequence,
                event_digest,
                command_id,
                request_digest,
                intent_digest: bytes_digest(intent)?,
                occurred_at: outbox_time,
                state,
                admission_digest: bytes_digest(admission)?,
            })
        }
        _ => return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt)),
    };
    Ok((event, outbox))
}

fn load_autonomy_receipt<C: GenericClient>(
    client: &mut C,
    stream_id_bytes: &[u8],
    stream: &VerifiedStream,
    sql: &str,
) -> PostgresTaskLedgerResult<VerifiedAutonomyReceiptState> {
    let rows = client
        .query(sql, &[&stream_id_bytes])
        .map_err(|database| map_database_error(&database))?;
    let mut untrusted = Vec::with_capacity(rows.len());
    for row in &rows {
        if row.len() != 24 {
            return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt));
        }
        untrusted.push(UntrustedAutonomyReceiptRow::new(
            row_digest(row, 0)?,
            parse_u64_text(&row_value::<String>(row, 1)?)?,
            row_digest(row, 2)?,
            row_value::<String>(row, 3)?,
            row_value::<String>(row, 4)?,
            row_value::<String>(row, 5)?,
            row_value::<String>(row, 6)?,
            row_value::<bool>(row, 7)?,
            row_value::<bool>(row, 8)?,
            row_value::<bool>(row, 9)?,
            row_value::<String>(row, 10)?,
            row_value::<String>(row, 11)?,
            row_value::<String>(row, 12)?,
            row_value::<Option<String>>(row, 13)?,
            row_value::<Option<String>>(row, 14)?,
            row_value::<String>(row, 15)?,
            row_digest(row, 16)?,
            row_digest(row, 17)?,
            row_digest(row, 18)?,
            optional_row_digest(row, 19)?,
            optional_row_digest(row, 20)?,
            row_value::<Option<String>>(row, 21)?
                .as_deref()
                .map(parse_u64_text)
                .transpose()?,
            row_digest(row, 22)?,
            row_digest(row, 23)?,
        ));
    }
    verify_untrusted_autonomy_receipt_rows(stream, &untrusted)
        .map_err(|ledger| map_ledger_error(&ledger))
}

fn load_autonomy_receipt_for_profile<C: GenericClient>(
    client: &mut C,
    stream_id_bytes: &[u8],
    stream: &VerifiedStream,
    sql_profile: TaskLedgerSqlProfile,
) -> PostgresTaskLedgerResult<VerifiedAutonomyReceiptState> {
    validate_autonomy_surface_for_profile(stream, sql_profile)?;
    let Some(sql) = sql_profile.autonomy_receipts_sql() else {
        return verify_untrusted_autonomy_receipt_rows(stream, &[])
            .map_err(|ledger| map_ledger_error(&ledger));
    };
    load_autonomy_receipt(client, stream_id_bytes, stream, sql)
}

fn load_foreman_records<C: GenericClient>(
    client: &mut C,
    stream_id_bytes: &[u8],
    stream: &VerifiedStream,
) -> PostgresTaskLedgerResult<Vec<VerifiedForemanSnapshotRecord>> {
    let rows = client
        .query(LEDGER_FOREMAN_SNAPSHOTS_SQL, &[&stream_id_bytes])
        .map_err(|database| map_database_error(&database))?;
    let mut untrusted = Vec::with_capacity(rows.len());
    for row in &rows {
        if row.len() != 33 {
            return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt));
        }
        let command_id_text = row_value::<String>(row, 3)?;
        let command = stream
            .commands()
            .iter()
            .find(|record| record.request().command_id().as_str() == command_id_text)
            .ok_or_else(|| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))?;
        let epistemic = epistemic_from_foreman_row(row)?;
        let mut snapshot = ForemanSnapshot::new(
            row_value::<String>(row, 8)?,
            row_value::<String>(row, 9)?,
            row_value::<String>(row, 10)?,
            row_value::<String>(row, 11)?,
            row_value::<String>(row, 12)?,
            row_value::<String>(row, 13)?,
            ForemanState::from_persisted(&row_value::<String>(row, 14)?)
                .map_err(|_| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))?,
            row_value::<Option<String>>(row, 15)?,
            row_value::<String>(row, 16)?,
            row_value::<String>(row, 17)?,
            row_value::<String>(row, 18)?,
            parse_u64_text(&row_value::<String>(row, 19)?)?,
        )
        .map_err(|_| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))?;
        if let Some(epistemic) = epistemic {
            snapshot = snapshot
                .with_epistemic(epistemic)
                .map_err(|_| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))?;
        }
        if row_value::<String>(row, 6)? != snapshot.schema() {
            return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt));
        }
        untrusted.push(UntrustedForemanSnapshotRow::new(
            row_value::<String>(row, 5)?,
            row_digest(row, 0)?,
            row_digest(row, 2)?,
            CommandId::new(command_id_text)
                .map_err(|_| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))?,
            row_digest(row, 4)?,
            row_digest(row, 7)?,
            snapshot,
            command.request().expected_head().clone(),
        ));
    }
    verify_untrusted_foreman_snapshot_rows(stream, &untrusted)
        .map_err(|ledger| map_ledger_error(&ledger))
}

#[allow(clippy::type_complexity)]
fn epistemic_from_foreman_row(row: &Row) -> PostgresTaskLedgerResult<Option<EpistemicReferences>> {
    let values = (
        row_value::<Option<String>>(row, 20)?,
        row_value::<Option<Vec<String>>>(row, 21)?,
        row_value::<Option<Vec<String>>>(row, 22)?,
        row_value::<Option<String>>(row, 23)?,
        row_value::<Option<Vec<String>>>(row, 24)?,
        row_value::<Option<Vec<String>>>(row, 25)?,
        row_value::<Option<Vec<String>>>(row, 26)?,
        row_value::<Option<String>>(row, 27)?,
        row_value::<Option<String>>(row, 28)?,
        row_value::<Option<String>>(row, 29)?,
        row_value::<Option<String>>(row, 30)?,
        row_value::<Option<String>>(row, 31)?,
        row_value::<Option<String>>(row, 32)?,
    );
    match values {
        (None, None, None, None, None, None, None, None, None, None, None, None, None) => Ok(None),
        (
            Some(schema),
            Some(observed),
            Some(hypotheses),
            Some(confidence),
            Some(unknowns),
            Some(evidence),
            Some(counterevidence),
            Some(checked_at),
            Some(expires_at),
            Some(refresh_trigger),
            Some(decision),
            Some(probe),
            Some(falsifier),
        ) if schema == "lattice.foreman-epistemic/1.0" => EpistemicReferences::new(
            observed,
            hypotheses,
            Confidence::from_persisted(&confidence)
                .map_err(|_| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))?,
            unknowns,
            evidence,
            counterevidence,
            checked_at,
            expires_at,
            RefreshTrigger::from_persisted(&refresh_trigger)
                .map_err(|_| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))?,
            decision,
            probe,
            falsifier,
        )
        .map(Some)
        .map_err(|_| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt)),
        _ => Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt)),
    }
}

fn validate_autonomy_surface_for_profile(
    stream: &VerifiedStream,
    sql_profile: TaskLedgerSqlProfile,
) -> PostgresTaskLedgerResult<()> {
    if sql_profile.supports_autonomy() {
        return Ok(());
    }
    for event in stream.events() {
        let required_profile = classify_task_created_profile(event)
            .map_err(|ledger| map_ledger_error(&ledger))?
            == Some(TaskCreatedProfile::AutonomyReceiptRequiredV1);
        if required_profile || event.kind() == LedgerEventKind::AutonomyReceiptRecorded {
            return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt));
        }
    }
    Ok(())
}

fn parse_command_row(
    row: &Row,
    identity: &TaskLedgerStreamIdentity,
    stream_id: &ContentDigest,
    database_uuid: &str,
    store_persistence: &StorePersistenceEvidence,
) -> PostgresTaskLedgerResult<ParsedCommandRow> {
    if row.len() != 83 || row_digest(row, 0)? != *stream_id {
        return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt));
    }
    let command_id: String = row_value(row, 1)?;
    let request_digest = row_digest(row, 3)?;
    let expected_head = task_head(
        identity.clone(),
        stream_id.clone(),
        parse_u64_text(&row_value::<String>(row, 4)?)?,
        row_digest(row, 5)?,
        parse_u64_text(&row_value::<String>(row, 6)?)?,
        row_digest(row, 7)?,
        row_digest(row, 8)?,
    )?;
    let request = UntrustedAppendRequest {
        schema_version: row_value(row, 2)?,
        expected_head,
        command_id: command_id.clone(),
        correlation_id: row_value(row, 9)?,
        occurred_at: row_value(row, 10)?,
        kind: row_value(row, 11)?,
        actor_id: row_value(row, 12)?,
        action: row_value(row, 13)?,
        outcome: row_value(row, 14)?,
        reason_code: row_value(row, 15)?,
        subject_digest: row_digest(row, 16)?,
        diagnostic: optional_diagnostic(row, 17)?,
        resource_snapshot: resource_snapshot_from_row(row, 18, 19)?,
    };
    let before = task_head(
        identity.clone(),
        stream_id.clone(),
        parse_u64_text(&row_value::<String>(row, 26)?)?,
        row_digest(row, 27)?,
        parse_u64_text(&row_value::<String>(row, 28)?)?,
        row_digest(row, 29)?,
        row_digest(row, 30)?,
    )?;
    let after = task_head(
        identity.clone(),
        stream_id.clone(),
        parse_u64_text(&row_value::<String>(row, 31)?)?,
        row_digest(row, 32)?,
        parse_u64_text(&row_value::<String>(row, 33)?)?,
        row_digest(row, 34)?,
        row_digest(row, 35)?,
    )?;
    let outcome: String = row_value(row, 36)?;
    let retained_denial: String = row_value(row, 37)?;
    let retained_event_bytes: Vec<u8> = row_value(row, 38)?;
    let (denial_reason, event_digest) = match outcome.as_str() {
        "APPENDED" if retained_denial.is_empty() => {
            let event = bytes_digest(&retained_event_bytes)?;
            if event.as_str() == ZERO_DIGEST_TEXT {
                return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt));
            }
            (None, Some(event))
        }
        "DENIED" if !retained_denial.is_empty() => {
            if bytes_digest(&retained_event_bytes)?.as_str() != ZERO_DIGEST_TEXT {
                return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt));
            }
            (Some(retained_denial), None)
        }
        _ => return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt)),
    };
    let receipt_digest = row_digest(row, 39)?;
    let receipt = UntrustedCommandReceipt {
        schema_version: row_value(row, 25)?,
        command_id: command_id.clone(),
        request_digest,
        before,
        after,
        outcome,
        denial_reason,
        event_digest,
        receipt_digest,
    };
    let base_checkpoint =
        LedgerCheckpoint::from_retained(stream_id.clone(), RuntimeKind::Live, row_digest(row, 40)?);
    let result_checkpoint =
        LedgerCheckpoint::from_retained(stream_id.clone(), RuntimeKind::Live, row_digest(row, 41)?);
    let record_set_digest = row_digest(row, 42)?;
    let store_receipt =
        reconstruct_store_receipt(row, identity, stream_id, database_uuid, store_persistence)?;
    Ok(ParsedCommandRow {
        command_id: command_id.clone(),
        record: UntrustedCommandRecord {
            stream_id: stream_id.clone(),
            command_id,
            request,
            receipt,
            base_checkpoint,
            result_checkpoint,
        },
        record_set_digest,
        store_receipt,
    })
}

fn optional_diagnostic(
    row: &Row,
    index: usize,
) -> PostgresTaskLedgerResult<Option<CanonicalValue>> {
    let value: JsonValue = row_value(row, index)?;
    if value.is_null() {
        return Ok(None);
    }
    let diagnostic = diagnostic_from_json(&value)?;
    Ok(Some(diagnostic.value().clone()))
}

fn resource_snapshot_from_row(
    row: &Row,
    flag_index: usize,
    first_value_index: usize,
) -> PostgresTaskLedgerResult<Option<ResourceCounters>> {
    let present: bool = row_value(row, flag_index)?;
    let active_agents: String = row_value(row, first_value_index)?;
    let active_implementers: String = row_value(row, first_value_index + 1)?;
    let elapsed_seconds: String = row_value(row, first_value_index + 2)?;
    let attempt_number: String = row_value(row, first_value_index + 3)?;
    let used_model_calls: String = row_value(row, first_value_index + 4)?;
    let used_external_cost: String = row_value(row, first_value_index + 5)?;
    if !present {
        if active_agents != "0"
            || active_implementers != "0"
            || elapsed_seconds != "0"
            || attempt_number != "0"
            || used_model_calls != "0"
            || used_external_cost != "0"
        {
            return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt));
        }
        return Ok(None);
    }
    Ok(Some(resource_counters(
        &active_agents,
        &active_implementers,
        &elapsed_seconds,
        &attempt_number,
        &used_model_calls,
        used_external_cost,
    )?))
}

#[allow(clippy::too_many_lines)]
fn reconstruct_store_receipt(
    row: &Row,
    identity: &TaskLedgerStreamIdentity,
    stream_id: &ContentDigest,
    database_uuid: &str,
    store_persistence: &StorePersistenceEvidence,
) -> PostgresTaskLedgerResult<StoreTransactionReceipt> {
    let found: bool = row_value(row, 44)?;
    let version: i16 = row_value(row, 45)?;
    let producer_id: String = row_value(row, 46)?;
    let producer_version: String = row_value(row, 47)?;
    let runtime: String = row_value(row, 48)?;
    let durability: String = row_value(row, 49)?;
    let retained_database_uuid: String = row_value(row, 50)?;
    let database_identity_digest = row_digest(row, 51)?;
    let schema_version: i16 = row_value(row, 52)?;
    let manifest_sha256: String = row_value(row, 53)?;
    if !found
        || version != i16::try_from(STORE_CONTRACT_VERSION).unwrap_or_default()
        || producer_id != STORE_PRODUCER_ID
        || producer_version != STORE_PRODUCER_VERSION
        || runtime != "LIVE"
        || durability != StoreDurability::DurablePostgres.as_str()
        || retained_database_uuid != database_uuid
        || database_identity_digest != *store_persistence.database_identity_digest()
        || schema_version != i16::try_from(store_persistence.schema_version()).unwrap_or_default()
        || manifest_sha256 != FROZEN_STORE_MANIFEST_SHA256
    {
        return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt));
    }
    let project_id: String = row_value(row, 54)?;
    let snapshot_id: String = row_value(row, 55)?;
    let owner: String = row_value(row, 56)?;
    let aggregate = row_digest(row, 57)?;
    if project_id != identity.project_id().as_str()
        || snapshot_id != identity.project_snapshot_id().as_str()
        || owner != StoreRepositoryOwner::TaskLedger.as_str()
        || aggregate != *stream_id
    {
        return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt));
    }
    let scope = ledger_scope(identity, stream_id.clone())?;
    let authority = StoreAuthorityHead::new(
        RuntimeKind::Live,
        StoreDaemonInstanceId::new(row_value::<String>(row, 59)?)
            .map_err(|_| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))?,
        DaemonEpoch::new(positive_i64(row_value(row, 60)?)?)
            .map_err(|_| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))?,
        parse_admission(&row_value::<String>(row, 61)?)?,
        StoreAuthorityRevision::new(positive_i64(row_value(row, 62)?)?)
            .map_err(|_| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))?,
        row_digest(row, 63)?,
        row_digest(row, 64)?,
    )
    .map_err(|_| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))?;
    let expected = stored_physical_head(
        &scope,
        row_value(row, 65)?,
        &row_value::<Vec<u8>>(row, 66)?,
        &row_value::<Vec<u8>>(row, 67)?,
    )?;
    let mutation = StoreMutationCommitment::new(
        row_digest(row, 68)?,
        row_digest(row, 69)?,
        row_digest(row, 70)?,
        row_digest(row, 71)?,
        optional_row_digest(row, 72)?,
        optional_row_digest(row, 73)?,
    )
    .map_err(|_| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))?;
    let transaction_id = StoreTransactionId::new(row_value::<String>(row, 43)?)
        .map_err(|_| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))?;
    let request = StoreTransactionRequest::new(
        STORE_CONTRACT_VERSION,
        transaction_id,
        scope.clone(),
        authority,
        expected,
        mutation,
    )
    .map_err(|_| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))?;
    let retained_request_digest = row_digest(row, 58)?;
    let canonical_request_digest = request_digest(&request)
        .map_err(|_| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))?;
    if canonical_request_digest != retained_request_digest {
        return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt));
    }
    let disposition = match row_value::<String>(row, 74)?.as_str() {
        "APPLIED" => StoreReceiptDisposition::Applied,
        "STALE_PHYSICAL_HEAD" => StoreReceiptDisposition::StalePhysicalHead,
        _ => return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt)),
    };
    let before = stored_physical_head(
        &scope,
        row_value(row, 75)?,
        &row_value::<Vec<u8>>(row, 76)?,
        &row_value::<Vec<u8>>(row, 77)?,
    )?;
    let after = stored_physical_head(
        &scope,
        row_value(row, 78)?,
        &row_value::<Vec<u8>>(row, 79)?,
        &row_value::<Vec<u8>>(row, 80)?,
    )?;
    let receipt = build_live_receipt(
        request,
        store_persistence.clone(),
        retained_request_digest,
        before,
        after,
        disposition,
    )
    .map_err(|_| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))?;
    if receipt.transaction_digest() != &row_digest(row, 81)?
        || receipt.receipt_digest() != &row_digest(row, 82)?
    {
        return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt));
    }
    Ok(receipt)
}

fn optional_row_digest(row: &Row, index: usize) -> PostgresTaskLedgerResult<Option<ContentDigest>> {
    row_value::<Option<Vec<u8>>>(row, index)?
        .as_deref()
        .map(bytes_digest)
        .transpose()
}

fn positive_i64(value: i64) -> PostgresTaskLedgerResult<u64> {
    let value =
        u64::try_from(value).map_err(|_| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))?;
    if value == 0 {
        return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt));
    }
    Ok(value)
}

fn parse_admission(value: &str) -> PostgresTaskLedgerResult<RuntimeAdmissionMode> {
    match value {
        "ACTIVE" => Ok(RuntimeAdmissionMode::Active),
        "DRAINING" => Ok(RuntimeAdmissionMode::Draining),
        "CANARY" => Ok(RuntimeAdmissionMode::Canary),
        "STOPPED" => Ok(RuntimeAdmissionMode::Stopped),
        "RECONCILIATION_REQUIRED" => Ok(RuntimeAdmissionMode::ReconciliationRequired),
        _ => Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt)),
    }
}

fn validate_store_bindings(
    stream: &VerifiedStream,
    store_receipts: &BTreeMap<String, StoreTransactionReceipt>,
    record_set_digests: &BTreeMap<String, ContentDigest>,
) -> PostgresTaskLedgerResult<()> {
    if stream.commands().len() != store_receipts.len()
        || stream.commands().len() != record_set_digests.len()
    {
        return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt));
    }
    for command in stream.commands() {
        let command_id = command.request().command_id().as_str();
        let receipt = store_receipts
            .get(command_id)
            .ok_or_else(|| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))?;
        let retained_record_set = record_set_digests
            .get(command_id)
            .ok_or_else(|| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))?;
        let plan = plan_append(stream, command.request().clone())
            .map_err(|ledger| map_ledger_error(&ledger))?;
        if !plan.is_exact_retry() || plan.record_set_digest() != retained_record_set {
            return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt));
        }
        let outbox = stream
            .outboxes()
            .iter()
            .find(|outbox| outbox.command_id().as_str() == command_id);
        let reconstructed = store_request_for_plan(
            &plan,
            receipt.request().expected_authority().clone(),
            receipt.request().expected_head().clone(),
            outbox,
        )?;
        if receipt.disposition() != StoreReceiptDisposition::Applied
            || receipt.request() != &reconstructed
            || receipt.after_head().state_digest()
                != command.result_checkpoint().checkpoint_digest()
        {
            return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn run_execute_attempt(
    client: &mut Client,
    database_uuid: &str,
    store_persistence: &StorePersistenceEvidence,
    global_persistence: &PostgresTaskLedgerPersistenceEvidence,
    sql_profile: TaskLedgerSqlProfile,
    identity: &TaskLedgerStreamIdentity,
    stream_id_bytes: &[u8],
    command: AppendCommand,
    command_id: &str,
    expected_authority: StoreAuthorityHead,
    writer_authority: Option<&WriterLeaseAuthorityHead>,
    autonomy_plan: Option<&AutonomyReceiptAppendPlan>,
    foreman_plan: Option<&ForemanSnapshotAppendPlan>,
    submission: Option<&TaskSubmissionEnvelope>,
    ingress_claim: Option<&TaskIngressClaim>,
) -> Result<PostgresTaskLedgerExecution, AttemptFailure> {
    let Ok(global_schema_version) = i16::try_from(global_persistence.schema_version()) else {
        return Err(AttemptFailure::Terminal(error(
            PostgresTaskLedgerErrorKind::RetainedRowCorrupt,
        )));
    };
    let global_manifest_sha256 = global_persistence.manifest_digest().as_str().to_owned();
    let mut transaction = client
        .build_transaction()
        .isolation_level(IsolationLevel::Serializable)
        .start()
        .map_err(|database| classify_query_error(&database))?;
    if let Err(database) = transaction.batch_execute(WRITE_TRANSACTION_SETTINGS) {
        return rollback_attempt(transaction, classify_query_error(&database));
    }
    let retained_ingress_claim = if let Some(claim) = ingress_claim {
        if !sql_profile.supports_submission() {
            return rollback_attempt(
                transaction,
                AttemptFailure::Terminal(error(
                    PostgresTaskLedgerErrorKind::UnsupportedRetainedSchema,
                )),
            );
        }
        match prepare_task_ingress_claim(&mut transaction, claim) {
            Ok(retained) => retained,
            Err(prepare_error) => {
                return rollback_attempt(transaction, AttemptFailure::Terminal(prepare_error));
            }
        }
    } else {
        None
    };
    let retained_submission = if let Some(submission) = submission {
        if !sql_profile.supports_submission() {
            return rollback_attempt(
                transaction,
                AttemptFailure::Terminal(error(
                    PostgresTaskLedgerErrorKind::UnsupportedRetainedSchema,
                )),
            );
        }
        match prepare_task_submission(&mut transaction, submission) {
            Ok(retained) => retained,
            Err(prepare_error) => {
                return rollback_attempt(transaction, AttemptFailure::Terminal(prepare_error));
            }
        }
    } else {
        None
    };
    let prepared_command_id = retained_ingress_claim
        .as_ref()
        .map(|retained| retained.command_id.as_str())
        .or_else(|| {
            retained_submission
                .as_ref()
                .map(|retained| retained.command_id.as_str())
        })
        .unwrap_or(command_id);
    let prepare_params = global_profile_params(
        sql_profile,
        &global_schema_version,
        &global_manifest_sha256,
        [
            &stream_id_bytes as &(dyn ToSql + Sync),
            &prepared_command_id,
        ],
    );
    let prepare_row = match transaction.query_one(sql_profile.ledger_prepare_sql(), &prepare_params)
    {
        Ok(row) => row,
        Err(database) => return rollback_attempt(transaction, classify_query_error(&database)),
    };
    let prepare = match parse_ledger_prepare_row(&prepare_row) {
        Ok(prepare) => prepare,
        Err(prepare_error) => {
            return rollback_attempt(transaction, AttemptFailure::Terminal(prepare_error));
        }
    };
    let loaded = match load_verified_stream(
        &mut transaction,
        identity,
        stream_id_bytes,
        database_uuid,
        store_persistence,
        global_persistence,
        sql_profile,
    ) {
        Ok(loaded) => loaded,
        Err(load_error) => {
            return rollback_attempt(transaction, AttemptFailure::Terminal(load_error));
        }
    };
    if let Err(prepare_error) = validate_ledger_prepare(&prepare, &loaded, prepared_command_id) {
        return rollback_attempt(transaction, AttemptFailure::Terminal(prepare_error));
    }
    if submission.is_some() && (retained_ingress_claim.is_some() != retained_submission.is_some()) {
        return rollback_attempt(
            transaction,
            AttemptFailure::Terminal(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt)),
        );
    }
    if let Some(retained) = retained_ingress_claim.as_ref()
        && !ingress_claim_matches_loaded_stream(retained, &loaded.stream)
    {
        return rollback_attempt(
            transaction,
            AttemptFailure::Terminal(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt)),
        );
    }
    if let Some(retained) = retained_submission.as_ref() {
        if retained_ingress_claim.is_none()
            || submission != Some(&retained.submission)
            || !submission_matches_loaded_stream(retained, &loaded.stream)
        {
            return rollback_attempt(
                transaction,
                AttemptFailure::Terminal(error(PostgresTaskLedgerErrorKind::CommandSubstitution)),
            );
        }
        let Some(command_record) = loaded
            .stream
            .commands()
            .iter()
            .find(|record| record.request().command_id().as_str() == retained.command_id)
        else {
            return rollback_attempt(
                transaction,
                AttemptFailure::Terminal(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt)),
            );
        };
        if !submission_matches_command(&retained.submission, command_record.request())
            || command_record.receipt().request_digest() != &retained.request_digest
        {
            return rollback_attempt(
                transaction,
                AttemptFailure::Terminal(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt)),
            );
        }
        let Some(store_receipt) = loaded.store_receipts.get(&retained.command_id).cloned() else {
            return rollback_attempt(
                transaction,
                AttemptFailure::Terminal(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt)),
            );
        };
        let outbox_admission = loaded
            .stream
            .outboxes()
            .iter()
            .find(|outbox| outbox.command_id().as_str() == retained.command_id)
            .cloned();
        let execution = PostgresTaskLedgerExecution {
            receipt: command_record.receipt().clone(),
            result_checkpoint: command_record.result_checkpoint().clone(),
            outbox_admission,
            store_receipt,
            persistence: global_persistence.clone(),
            exact_retry: true,
        };
        return transaction
            .commit()
            .map(|()| execution)
            .map_err(|database| classify_commit_error(&database));
    }
    let foreman_records = if foreman_plan.is_some() {
        match load_foreman_records(&mut transaction, stream_id_bytes, &loaded.stream) {
            Ok(records) => Some(records),
            Err(load_error) => {
                return rollback_attempt(transaction, AttemptFailure::Terminal(load_error));
            }
        }
    } else {
        None
    };
    let plan = match plan_append(&loaded.stream, command) {
        Ok(plan) => plan,
        Err(ledger) => {
            return rollback_attempt(
                transaction,
                AttemptFailure::Terminal(map_plan_error(&ledger)),
            );
        }
    };
    let autonomy_receipt = autonomy_plan.map(AutonomyReceiptAppendPlan::receipt);
    if autonomy_plan
        .is_some_and(|submitted| !plan.is_exact_retry() && submitted.append_plan() != &plan)
    {
        return rollback_attempt(
            transaction,
            AttemptFailure::Terminal(error(PostgresTaskLedgerErrorKind::Malformed)),
        );
    }
    if foreman_plan.is_some_and(|submitted| submitted.ledger_plan() != &plan) {
        return rollback_attempt(
            transaction,
            AttemptFailure::Terminal(error(PostgresTaskLedgerErrorKind::Malformed)),
        );
    }

    if !sql_profile.supports_autonomy()
        && (autonomy_plan.is_some() || plan_uses_autonomy_surface(&plan))
    {
        return rollback_attempt(
            transaction,
            AttemptFailure::Terminal(error(PostgresTaskLedgerErrorKind::Malformed)),
        );
    }

    if !sql_profile.supports_foreman() && foreman_plan.is_some() {
        return rollback_attempt(
            transaction,
            AttemptFailure::Terminal(error(PostgresTaskLedgerErrorKind::Malformed)),
        );
    }

    if plan.is_exact_retry() {
        if ingress_claim.is_some() && retained_ingress_claim.is_none() {
            return rollback_attempt(
                transaction,
                AttemptFailure::Terminal(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt)),
            );
        }
        if submission.is_some()
            && retained_submission.as_ref().is_none_or(|retained| {
                Some(&retained.submission) != submission
                    || !submission_matches_loaded_stream(retained, &loaded.stream)
            })
        {
            return rollback_attempt(
                transaction,
                AttemptFailure::Terminal(error(PostgresTaskLedgerErrorKind::CommandSubstitution)),
            );
        }
        if autonomy_receipt.is_some_and(|receipt| {
            loaded.autonomy_state != VerifiedAutonomyReceiptState::RequiredComplete(receipt.clone())
        }) {
            return rollback_attempt(
                transaction,
                AttemptFailure::Terminal(error(PostgresTaskLedgerErrorKind::CommandSubstitution)),
            );
        }
        if loaded.record_set_digests.get(command_id) != Some(plan.record_set_digest()) {
            return rollback_attempt(
                transaction,
                AttemptFailure::Terminal(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt)),
            );
        }
        if foreman_plan.is_some()
            && !foreman_records.as_ref().is_some_and(|records| {
                records
                    .iter()
                    .any(|record| record.command_id().as_str() == command_id)
            })
        {
            return rollback_attempt(
                transaction,
                AttemptFailure::Terminal(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt)),
            );
        }
        let store_receipt = match loaded.store_receipts.get(command_id) {
            Some(receipt) => receipt.clone(),
            None => {
                return rollback_attempt(
                    transaction,
                    AttemptFailure::Terminal(error(
                        PostgresTaskLedgerErrorKind::RetainedRowCorrupt,
                    )),
                );
            }
        };
        let outbox_admission = loaded
            .stream
            .outboxes()
            .iter()
            .find(|outbox| outbox.command_id().as_str() == command_id)
            .cloned();
        let execution = PostgresTaskLedgerExecution {
            receipt: plan.receipt().clone(),
            result_checkpoint: plan.command_record().result_checkpoint().clone(),
            outbox_admission,
            store_receipt,
            persistence: global_persistence.clone(),
            exact_retry: true,
        };
        return transaction
            .commit()
            .map(|()| execution)
            .map_err(|database| classify_commit_error(&database));
    }

    if retained_submission.is_some() || retained_ingress_claim.is_some() {
        return rollback_attempt(
            transaction,
            AttemptFailure::Terminal(error(PostgresTaskLedgerErrorKind::CommandSubstitution)),
        );
    }

    if let Some(writer_authority) = writer_authority
        && let Err(assertion_error) = assert_writer_authority(&mut transaction, writer_authority)
    {
        return rollback_attempt(transaction, assertion_error);
    }

    if expected_authority.runtime() != RuntimeKind::Live {
        return rollback_attempt(
            transaction,
            AttemptFailure::Terminal(error(PostgresTaskLedgerErrorKind::AuthorityMismatch)),
        );
    }
    if expected_authority.admission() != RuntimeAdmissionMode::Active {
        return rollback_attempt(
            transaction,
            AttemptFailure::Terminal(error(PostgresTaskLedgerErrorKind::AdmissionDenied)),
        );
    }
    let request = match store_request_for_plan(
        &plan,
        expected_authority,
        loaded.physical_head.clone(),
        plan.new_outbox(),
    ) {
        Ok(request) => request,
        Err(request_error) => {
            return rollback_attempt(transaction, AttemptFailure::Terminal(request_error));
        }
    };
    let Ok(canonical_request_digest) = request_digest(&request) else {
        return rollback_attempt(
            transaction,
            AttemptFailure::Terminal(error(PostgresTaskLedgerErrorKind::Malformed)),
        );
    };
    let Ok(genesis) = genesis_head(RuntimeKind::Live, request.scope().clone()) else {
        return rollback_attempt(
            transaction,
            AttemptFailure::Terminal(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt)),
        );
    };
    let store_values = match StoreSqlValues::new(&request, &canonical_request_digest, &genesis) {
        Ok(values) => values,
        Err(values_error) => {
            return rollback_attempt(transaction, AttemptFailure::Terminal(values_error));
        }
    };
    let store_prepare_params = global_profile_params(
        sql_profile,
        &global_schema_version,
        &global_manifest_sha256,
        store_values.params(),
    );
    let store_prepare_row =
        match transaction.query_one(sql_profile.store_prepare_sql(), &store_prepare_params) {
            Ok(row) => row,
            Err(database) => return rollback_attempt(transaction, classify_query_error(&database)),
        };
    let store_prepare = match parse_store_prepare_row(&store_prepare_row) {
        Ok(prepare) => prepare,
        Err(prepare_error) => {
            return rollback_attempt(transaction, AttemptFailure::Terminal(prepare_error));
        }
    };
    let store_receipt = match build_new_store_receipt(
        &store_prepare,
        &request,
        &canonical_request_digest,
        &loaded.physical_head,
        database_uuid,
        store_persistence,
        global_persistence,
    ) {
        Ok(receipt) => receipt,
        Err(receipt_error) => {
            return rollback_attempt(transaction, AttemptFailure::Terminal(receipt_error));
        }
    };
    match call_store_finalize(
        &mut transaction,
        &store_values,
        database_uuid,
        store_persistence,
        sql_profile,
        global_schema_version,
        &global_manifest_sha256,
        &store_receipt,
    ) {
        Ok(status) if status == "FINALIZED" => {}
        Ok(_) => {
            return rollback_attempt(
                transaction,
                AttemptFailure::Terminal(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt)),
            );
        }
        Err(StoreCallFailure::Database(database)) => {
            return rollback_attempt(transaction, classify_query_error(&database));
        }
        Err(StoreCallFailure::Invalid(finalize_error)) => {
            return rollback_attempt(transaction, AttemptFailure::Terminal(finalize_error));
        }
    }
    let ledger_status = if submission.is_some() {
        let Some(finalize_sql) = sql_profile.general_ledger_finalize_sql() else {
            return rollback_attempt(
                transaction,
                AttemptFailure::Terminal(error(
                    PostgresTaskLedgerErrorKind::UnsupportedRetainedSchema,
                )),
            );
        };
        let ledger_values = match GeneralLedgerFinalizeValues::new(&plan, &store_receipt) {
            Ok(values) => values,
            Err(values_error) => {
                return rollback_attempt(transaction, AttemptFailure::Terminal(values_error));
            }
        };
        let ledger_finalize_params = global_profile_params(
            sql_profile,
            &global_schema_version,
            &global_manifest_sha256,
            ledger_values.params(),
        );
        match transaction.query_one(finalize_sql, &ledger_finalize_params) {
            Ok(row) => match row_value::<String>(&row, 0) {
                Ok(status) => status,
                Err(row_error) => {
                    return rollback_attempt(transaction, AttemptFailure::Terminal(row_error));
                }
            },
            Err(database) => return rollback_attempt(transaction, classify_query_error(&database)),
        }
    } else {
        let ledger_values = match LedgerFinalizeValues::new(&plan, &store_receipt) {
            Ok(values) => values,
            Err(values_error) => {
                return rollback_attempt(transaction, AttemptFailure::Terminal(values_error));
            }
        };
        let ledger_finalize_params = global_profile_params(
            sql_profile,
            &global_schema_version,
            &global_manifest_sha256,
            ledger_values.params(),
        );
        match transaction.query_one(sql_profile.ledger_finalize_sql(), &ledger_finalize_params) {
            Ok(row) => match row_value::<String>(&row, 0) {
                Ok(status) => status,
                Err(row_error) => {
                    return rollback_attempt(transaction, AttemptFailure::Terminal(row_error));
                }
            },
            Err(database) => return rollback_attempt(transaction, classify_query_error(&database)),
        }
    };
    if ledger_status != "FINALIZED" {
        return rollback_attempt(
            transaction,
            AttemptFailure::Terminal(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt)),
        );
    }
    if let Some(receipt) = autonomy_receipt {
        let event = match plan.new_event() {
            Some(event)
                if event.kind()
                    == lattice_task_ledger::LedgerEventKind::AutonomyReceiptRecorded =>
            {
                event
            }
            _ => {
                return rollback_attempt(
                    transaction,
                    AttemptFailure::Terminal(error(PostgresTaskLedgerErrorKind::Malformed)),
                );
            }
        };
        if receipt.stream_id() != event.stream_id()
            || receipt.event_sequence() != event.sequence()
            || receipt.event_digest() != event.event_digest()
            || receipt.receipt_digest() != event.subject_digest()
        {
            return rollback_attempt(
                transaction,
                AttemptFailure::Terminal(error(PostgresTaskLedgerErrorKind::Malformed)),
            );
        }
        let Some(record_sql) = sql_profile.autonomy_record_sql() else {
            return rollback_attempt(
                transaction,
                AttemptFailure::Terminal(error(PostgresTaskLedgerErrorKind::Malformed)),
            );
        };
        let values = AutonomySqlValues::new(receipt);
        let status = match transaction.query_one(record_sql, &values.params()) {
            Ok(row) => match row_value::<String>(&row, 0) {
                Ok(status) => status,
                Err(row_error) => {
                    return rollback_attempt(transaction, AttemptFailure::Terminal(row_error));
                }
            },
            Err(database) => {
                return rollback_attempt(transaction, classify_query_error(&database));
            }
        };
        if status != "RECORDED" {
            return rollback_attempt(
                transaction,
                AttemptFailure::Terminal(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt)),
            );
        }
    } else if plan.new_event().is_some_and(|event| {
        event.kind() == lattice_task_ledger::LedgerEventKind::AutonomyReceiptRecorded
    }) {
        return rollback_attempt(
            transaction,
            AttemptFailure::Terminal(error(PostgresTaskLedgerErrorKind::Malformed)),
        );
    }
    if let Some(submitted) = foreman_plan {
        let Some(record) = submitted.new_record() else {
            return rollback_attempt(
                transaction,
                AttemptFailure::Terminal(error(PostgresTaskLedgerErrorKind::Malformed)),
            );
        };
        let Some(writer_authority) = writer_authority else {
            return rollback_attempt(
                transaction,
                AttemptFailure::Terminal(error(PostgresTaskLedgerErrorKind::AuthorityMismatch)),
            );
        };
        let values = match ForemanSqlValues::new(writer_authority, record) {
            Ok(values) => values,
            Err(values_error) => {
                return rollback_attempt(transaction, AttemptFailure::Terminal(values_error));
            }
        };
        let status = match transaction
            .query_one(LEDGER_RECORD_FOREMAN_SNAPSHOT_SQL, &values.params())
        {
            Ok(row) => match row_value::<String>(&row, 0) {
                Ok(status) => status,
                Err(row_error) => {
                    return rollback_attempt(transaction, AttemptFailure::Terminal(row_error));
                }
            },
            Err(database) => return rollback_attempt(transaction, classify_query_error(&database)),
        };
        if status != "RECORDED" {
            return rollback_attempt(
                transaction,
                AttemptFailure::Terminal(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt)),
            );
        }
    } else if plan.new_event().is_some_and(|event| {
        event.kind() == lattice_task_ledger::LedgerEventKind::ForemanSnapshotRecorded
    }) {
        return rollback_attempt(
            transaction,
            AttemptFailure::Terminal(error(PostgresTaskLedgerErrorKind::Malformed)),
        );
    }
    if let Some(claim) = ingress_claim {
        let Some(event) = plan.new_event() else {
            return rollback_attempt(
                transaction,
                AttemptFailure::Terminal(error(PostgresTaskLedgerErrorKind::Malformed)),
            );
        };
        if let Err(record_error) = record_task_ingress_claim(&mut transaction, claim, event) {
            return rollback_attempt(transaction, record_error);
        }
    }
    if let Some(submission) = submission {
        let Some(event) = plan.new_event() else {
            return rollback_attempt(
                transaction,
                AttemptFailure::Terminal(error(PostgresTaskLedgerErrorKind::Malformed)),
            );
        };
        if let Err(record_error) = record_task_submission(&mut transaction, submission, event) {
            return rollback_attempt(transaction, AttemptFailure::Terminal(record_error));
        }
    }
    let reloaded = match load_verified_stream(
        &mut transaction,
        loaded.stream.identity(),
        stream_id_bytes,
        database_uuid,
        store_persistence,
        global_persistence,
        sql_profile,
    ) {
        Ok(reloaded) => reloaded,
        Err(load_error) => {
            return rollback_attempt(transaction, AttemptFailure::Terminal(load_error));
        }
    };
    let planned_state = match apply_append_plan(&loaded.stream, &plan) {
        Ok(state) => state,
        Err(ledger) => {
            return rollback_attempt(
                transaction,
                AttemptFailure::Terminal(map_ledger_error(&ledger)),
            );
        }
    };
    let reloaded_foreman_records = if foreman_plan.is_some() {
        match load_foreman_records(&mut transaction, stream_id_bytes, &reloaded.stream) {
            Ok(records) => Some(records),
            Err(load_error) => {
                return rollback_attempt(transaction, AttemptFailure::Terminal(load_error));
            }
        }
    } else {
        None
    };
    if reloaded.stream != planned_state
        || reloaded.retained_checkpoint != *plan.next_checkpoint()
        || reloaded.physical_head != *store_receipt.after_head()
        || reloaded.store_receipts.get(command_id) != Some(&store_receipt)
        || reloaded.record_set_digests.get(command_id) != Some(plan.record_set_digest())
        || autonomy_receipt.is_some_and(|receipt| {
            reloaded.autonomy_state
                != VerifiedAutonomyReceiptState::RequiredComplete(receipt.clone())
        })
        || foreman_plan.is_some_and(|submitted| {
            submitted.new_record().is_none_or(|record| {
                !reloaded_foreman_records
                    .as_ref()
                    .is_some_and(|records| records.iter().any(|retained| retained == record))
            })
        })
        || ingress_claim.is_some_and(|expected| {
            load_task_ingress_claim_by_request(&mut transaction, expected)
                .ok()
                .flatten()
                .is_none_or(|retained| {
                    !ingress_claim_matches_loaded_stream(&retained, &reloaded.stream)
                })
        })
        || submission.is_some_and(|expected| {
            load_task_submission_by_ref(&mut transaction, expected.task_ref())
                .ok()
                .flatten()
                .is_none_or(|retained| {
                    retained.submission != *expected
                        || !submission_matches_loaded_stream(&retained, &reloaded.stream)
                })
        })
    {
        return rollback_attempt(
            transaction,
            AttemptFailure::Terminal(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt)),
        );
    }
    let execution = PostgresTaskLedgerExecution {
        receipt: plan.receipt().clone(),
        result_checkpoint: plan.next_checkpoint().clone(),
        outbox_admission: plan.new_outbox().cloned(),
        store_receipt,
        persistence: global_persistence.clone(),
        exact_retry: false,
    };
    transaction
        .commit()
        .map(|()| execution)
        .map_err(|database| classify_commit_error(&database))
}

fn autonomy_plan_matches_store_authority(
    plan: &AutonomyReceiptAppendPlan,
    expected_authority: &StoreAuthorityHead,
) -> bool {
    plan.receipt().store_authority_head_digest() == expected_authority.head_digest()
}

fn plan_uses_autonomy_surface(plan: &LedgerAppendPlan) -> bool {
    plan.new_event().is_some_and(|event| {
        event.kind() == lattice_task_ledger::LedgerEventKind::AutonomyReceiptRecorded
            || classify_task_created_profile(event)
                == Ok(Some(TaskCreatedProfile::AutonomyReceiptRequiredV1))
    })
}

fn writer_authority_matches_identity(
    authority: &WriterLeaseAuthorityHead,
    identity: &TaskLedgerStreamIdentity,
) -> bool {
    let lease = authority.identity();
    authority.runtime() == RuntimeKind::Live
        && authority.status() == WriterLeaseStatus::Active
        && authority.runtime_admission() == RuntimeAdmissionMode::Active
        && lease.project_id() == identity.project_id()
        && lease.project_snapshot_id() == identity.project_snapshot_id()
        && lease.task_id() == identity.task_id()
        && lease.task_revision() == identity.task_revision()
        && identity
            .task_spec_digest()
            .is_some_and(|digest| lease.task_spec_digest() == digest)
}

fn assert_writer_authority(
    transaction: &mut Transaction<'_>,
    authority: &WriterLeaseAuthorityHead,
) -> Result<(), AttemptFailure> {
    let identity = authority.identity();
    let receipt_digest =
        digest_bytes(authority.receipt_digest()).map_err(AttemptFailure::Terminal)?;
    let task_spec_digest =
        digest_bytes(identity.task_spec_digest()).map_err(AttemptFailure::Terminal)?;
    let holder_process_start_identity =
        digest_bytes(identity.holder_process_start_identity()).map_err(AttemptFailure::Terminal)?;
    let holder_process_id = i64::try_from(identity.holder_process_id().get()).map_err(|_| {
        AttemptFailure::Terminal(error(PostgresTaskLedgerErrorKind::AuthorityMismatch))
    })?;
    let daemon_epoch = i64::try_from(identity.daemon_epoch().get()).map_err(|_| {
        AttemptFailure::Terminal(error(PostgresTaskLedgerErrorKind::AuthorityMismatch))
    })?;
    let fencing_token = i64::try_from(identity.fencing_token().get()).map_err(|_| {
        AttemptFailure::Terminal(error(PostgresTaskLedgerErrorKind::AuthorityMismatch))
    })?;
    let asserted = transaction
        .query_one(
            WRITER_LEASE_ASSERT_CURRENT_SQL,
            &[
                &identity.project_id().as_str(),
                &identity.project_snapshot_id().as_str(),
                &identity.task_id().as_str(),
                &identity.task_revision(),
                &task_spec_digest,
                &identity.attempt_id().as_str(),
                &identity.lease_id(),
                &identity.lease_holder_id(),
                &identity.worktree_id(),
                &holder_process_id,
                &holder_process_start_identity,
                &identity.daemon_instance_id(),
                &daemon_epoch,
                &fencing_token,
                &receipt_digest,
            ],
        )
        .and_then(|row| row.try_get::<_, bool>(0))
        .map_err(|database| classify_writer_assert_error(&database))?;
    if !asserted {
        return Err(AttemptFailure::Terminal(error(
            PostgresTaskLedgerErrorKind::AuthorityMismatch,
        )));
    }
    Ok(())
}

fn classify_writer_assert_error(database: &PostgresError) -> AttemptFailure {
    if database.code() == Some(&SqlState::T_R_SERIALIZATION_FAILURE) {
        AttemptFailure::Retryable
    } else if matches!(database.code().map(SqlState::code), Some("LWL02" | "LWL04")) {
        AttemptFailure::Terminal(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))
    } else if matches!(database.code().map(SqlState::code), Some("LWL03" | "LWL05")) {
        AttemptFailure::Terminal(error(PostgresTaskLedgerErrorKind::AuthorityMismatch))
    } else {
        AttemptFailure::Terminal(error(PostgresTaskLedgerErrorKind::Unavailable))
    }
}

fn parse_ledger_prepare_row(row: &Row) -> PostgresTaskLedgerResult<LedgerPrepareRow> {
    if row.len() != 9 {
        return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt));
    }
    Ok(LedgerPrepareRow {
        stream_found: row_value(row, 0)?,
        command_found: row_value(row, 1)?,
        retained_request_digest: optional_row_digest(row, 2)?,
        retained_receipt_digest: optional_row_digest(row, 3)?,
        retained_base_checkpoint_digest: optional_row_digest(row, 4)?,
        retained_result_checkpoint_digest: optional_row_digest(row, 5)?,
        retained_store_transaction_id: row_value(row, 6)?,
        terminal_found: row_value(row, 7)?,
        physical_state_digest: optional_row_digest(row, 8)?,
    })
}

fn validate_ledger_prepare(
    prepare: &LedgerPrepareRow,
    loaded: &LoadedStream,
    command_id: &str,
) -> PostgresTaskLedgerResult<()> {
    let persisted = !loaded.stream.commands().is_empty();
    if prepare.stream_found != persisted {
        return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt));
    }
    let command = loaded
        .stream
        .commands()
        .iter()
        .find(|record| record.request().command_id().as_str() == command_id);
    if prepare.command_found != command.is_some() {
        return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt));
    }
    match command {
        Some(command) => {
            let store = loaded
                .store_receipts
                .get(command_id)
                .ok_or_else(|| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))?;
            if prepare.retained_request_digest.as_ref() != Some(command.receipt().request_digest())
                || prepare.retained_receipt_digest.as_ref()
                    != Some(command.receipt().receipt_digest())
                || prepare.retained_base_checkpoint_digest.as_ref()
                    != Some(command.base_checkpoint().checkpoint_digest())
                || prepare.retained_result_checkpoint_digest.as_ref()
                    != Some(command.result_checkpoint().checkpoint_digest())
                || prepare.retained_store_transaction_id.as_deref()
                    != Some(store.request().transaction_id().as_str())
                || !prepare.terminal_found
            {
                return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt));
            }
        }
        None => {
            if prepare.retained_request_digest.is_some()
                || prepare.retained_receipt_digest.is_some()
                || prepare.retained_base_checkpoint_digest.is_some()
                || prepare.retained_result_checkpoint_digest.is_some()
                || prepare.retained_store_transaction_id.is_some()
                || prepare.terminal_found
            {
                return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt));
            }
        }
    }
    let expected_physical = persisted.then(|| loaded.physical_head.state_digest());
    if prepare.physical_state_digest.as_ref() != expected_physical {
        return Err(error(PostgresTaskLedgerErrorKind::PhysicalStateMismatch));
    }
    Ok(())
}

fn classify_query_error(database: &PostgresError) -> AttemptFailure {
    if database
        .as_db_error()
        .is_some_and(|db_error| retryable_sqlstate(db_error.code().code(), db_error.constraint()))
    {
        AttemptFailure::Retryable
    } else {
        AttemptFailure::Terminal(map_database_error(database))
    }
}

fn classify_commit_error(database: &PostgresError) -> AttemptFailure {
    let db_error = database.as_db_error();
    match commit_failure_class(
        db_error.map(|value| value.code().code()),
        db_error.and_then(|value| value.constraint()),
    ) {
        CommitFailureClass::Retryable => AttemptFailure::Retryable,
        CommitFailureClass::OutcomeUnknown => AttemptFailure::CommitOutcomeUnknown,
        CommitFailureClass::Terminal => AttemptFailure::Terminal(map_database_error(database)),
    }
}

fn commit_failure_class(code: Option<&str>, constraint: Option<&str>) -> CommitFailureClass {
    match code {
        Some(code) if retryable_sqlstate(code, constraint) => CommitFailureClass::Retryable,
        Some(_) => CommitFailureClass::Terminal,
        None => CommitFailureClass::OutcomeUnknown,
    }
}

fn rollback_attempt(
    transaction: Transaction<'_>,
    failure: AttemptFailure,
) -> Result<PostgresTaskLedgerExecution, AttemptFailure> {
    match transaction.rollback() {
        Ok(()) => Err(failure),
        Err(_) => Err(AttemptFailure::Terminal(error(
            PostgresTaskLedgerErrorKind::TransactionFailed,
        ))),
    }
}

struct AutonomySqlValues {
    stream_id: Vec<u8>,
    event_sequence: String,
    event_digest: Vec<u8>,
    receipt_schema_version: String,
    intent_version: String,
    task_kind: String,
    risk_class: String,
    execution_preapproved: bool,
    requires_new_authority: bool,
    irreversible_or_high_risk: bool,
    observed_task_state: String,
    disposition: String,
    decision_reason: String,
    model: Option<String>,
    verification: Option<String>,
    authority_mode: String,
    process_start_authority_digest: Vec<u8>,
    ingress_profile_adapter_commitment: Vec<u8>,
    store_authority_head_digest: Vec<u8>,
    writer_lease_receipt_digest: Option<Vec<u8>>,
    writer_lease_head_digest: Option<Vec<u8>>,
    writer_fencing_token: Option<String>,
    authority_digest: Vec<u8>,
    receipt_digest: Vec<u8>,
}

impl AutonomySqlValues {
    fn new(receipt: &VerifiedAutonomyReceipt) -> Self {
        let row = receipt.to_untrusted();
        Self {
            stream_id: digest_bytes(row.stream_id()).expect("verified digest"),
            event_sequence: row.event_sequence().to_string(),
            event_digest: digest_bytes(row.event_digest()).expect("verified digest"),
            receipt_schema_version: row.receipt_schema_version().to_owned(),
            intent_version: row.intent_version().to_owned(),
            task_kind: row.task_kind().to_owned(),
            risk_class: row.risk_class().to_owned(),
            execution_preapproved: row.execution_preapproved(),
            requires_new_authority: row.requires_new_authority(),
            irreversible_or_high_risk: row.irreversible_or_high_risk(),
            observed_task_state: row.observed_task_state().to_owned(),
            disposition: row.disposition().to_owned(),
            decision_reason: row.decision_reason().to_owned(),
            model: row.model().map(str::to_owned),
            verification: row.verification().map(str::to_owned),
            authority_mode: row.authority_mode().to_owned(),
            process_start_authority_digest: digest_bytes(row.process_start_authority_digest())
                .expect("validated digest"),
            ingress_profile_adapter_commitment: digest_bytes(
                row.ingress_profile_adapter_commitment(),
            )
            .expect("verified digest"),
            store_authority_head_digest: digest_bytes(row.store_authority_head_digest())
                .expect("validated digest"),
            writer_lease_receipt_digest: row
                .writer_lease_receipt_digest()
                .map(|value| digest_bytes(value).expect("validated digest")),
            writer_lease_head_digest: row
                .writer_lease_head_digest()
                .map(|value| digest_bytes(value).expect("validated digest")),
            writer_fencing_token: row.writer_fencing_token().map(|value| value.to_string()),
            authority_digest: digest_bytes(row.authority_digest()).expect("verified digest"),
            receipt_digest: digest_bytes(row.receipt_digest()).expect("verified digest"),
        }
    }

    fn params(&self) -> [&(dyn ToSql + Sync); 24] {
        [
            &self.stream_id,
            &self.event_sequence,
            &self.event_digest,
            &self.receipt_schema_version,
            &self.intent_version,
            &self.task_kind,
            &self.risk_class,
            &self.execution_preapproved,
            &self.requires_new_authority,
            &self.irreversible_or_high_risk,
            &self.observed_task_state,
            &self.disposition,
            &self.decision_reason,
            &self.model,
            &self.verification,
            &self.authority_mode,
            &self.process_start_authority_digest,
            &self.ingress_profile_adapter_commitment,
            &self.store_authority_head_digest,
            &self.writer_lease_receipt_digest,
            &self.writer_lease_head_digest,
            &self.writer_fencing_token,
            &self.authority_digest,
            &self.receipt_digest,
        ]
    }
}

struct ForemanSqlValues {
    writer_project_id: String,
    writer_project_snapshot_id: String,
    writer_task_id: String,
    writer_task_revision: String,
    writer_task_spec_digest: Vec<u8>,
    writer_attempt_id: String,
    writer_lease_id: String,
    writer_lease_holder_id: String,
    writer_worktree_id: String,
    writer_holder_process_id: i64,
    writer_holder_process_start_identity: Vec<u8>,
    writer_daemon_instance_id: String,
    writer_daemon_epoch: i64,
    writer_fencing_token: i64,
    writer_receipt_digest: Vec<u8>,
    stream_id: Vec<u8>,
    event_sequence: String,
    event_digest: Vec<u8>,
    command_id: String,
    request_digest: Vec<u8>,
    record_schema: String,
    payload_schema: String,
    payload_digest: Vec<u8>,
    worker_id: String,
    thread_id: String,
    task_id: String,
    branch_ref: String,
    worktree_ref: String,
    head_sha1: String,
    foreman_state: String,
    blocker_ref: Option<String>,
    heartbeat_digest_ref: String,
    authority_digest_ref: String,
    evidence_digest_ref: String,
    generation: String,
    epistemic_schema: Option<String>,
    observed_fact_refs: Option<Vec<String>>,
    hypothesis_refs: Option<Vec<String>>,
    confidence: Option<String>,
    unknown_refs: Option<Vec<String>>,
    evidence_refs: Option<Vec<String>>,
    counterevidence_refs: Option<Vec<String>>,
    checked_at: Option<String>,
    expires_at: Option<String>,
    refresh_trigger: Option<String>,
    decision_ref: Option<String>,
    probe_ref: Option<String>,
    falsifier_ref: Option<String>,
}

impl ForemanSqlValues {
    fn new(
        authority: &WriterLeaseAuthorityHead,
        record: &VerifiedForemanSnapshotRecord,
    ) -> PostgresTaskLedgerResult<Self> {
        let identity = authority.identity();
        let snapshot = record.snapshot();
        let epistemic = snapshot.epistemic();
        Ok(Self {
            writer_project_id: identity.project_id().as_str().to_owned(),
            writer_project_snapshot_id: identity.project_snapshot_id().as_str().to_owned(),
            writer_task_id: identity.task_id().as_str().to_owned(),
            writer_task_revision: identity.task_revision().to_owned(),
            writer_task_spec_digest: digest_bytes(identity.task_spec_digest())?,
            writer_attempt_id: identity.attempt_id().as_str().to_owned(),
            writer_lease_id: identity.lease_id().to_owned(),
            writer_lease_holder_id: identity.lease_holder_id().to_owned(),
            writer_worktree_id: identity.worktree_id().to_owned(),
            writer_holder_process_id: signed_i64(identity.holder_process_id().get())?,
            writer_holder_process_start_identity: digest_bytes(
                identity.holder_process_start_identity(),
            )?,
            writer_daemon_instance_id: identity.daemon_instance_id().to_owned(),
            writer_daemon_epoch: signed_i64(identity.daemon_epoch().get())?,
            writer_fencing_token: signed_i64(identity.fencing_token().get())?,
            writer_receipt_digest: digest_bytes(authority.receipt_digest())?,
            stream_id: digest_bytes(record.stream_id())?,
            event_sequence: record.event_sequence().to_string(),
            event_digest: digest_bytes(record.event_digest())?,
            command_id: record.command_id().as_str().to_owned(),
            request_digest: digest_bytes(record.request_digest())?,
            record_schema: FOREMAN_RECORD_SCHEMA.to_owned(),
            payload_schema: snapshot.schema().to_owned(),
            payload_digest: digest_bytes(record.payload_digest())?,
            worker_id: snapshot.worker().to_owned(),
            thread_id: snapshot.thread().to_owned(),
            task_id: snapshot.task().to_owned(),
            branch_ref: snapshot.branch().to_owned(),
            worktree_ref: snapshot.worktree().to_owned(),
            head_sha1: snapshot.head().to_owned(),
            foreman_state: snapshot.state().as_str().to_owned(),
            blocker_ref: snapshot.blocker().map(str::to_owned),
            heartbeat_digest_ref: snapshot.heartbeat().to_owned(),
            authority_digest_ref: snapshot.authority().to_owned(),
            evidence_digest_ref: snapshot.evidence().to_owned(),
            generation: snapshot.generation().to_string(),
            epistemic_schema: epistemic.map(|value| value.schema().to_owned()),
            observed_fact_refs: epistemic.map(|value| value.observed_facts().to_vec()),
            hypothesis_refs: epistemic.map(|value| value.hypotheses().to_vec()),
            confidence: epistemic.map(|value| value.confidence().as_str().to_owned()),
            unknown_refs: epistemic.map(|value| value.unknowns().to_vec()),
            evidence_refs: epistemic.map(|value| value.evidence().to_vec()),
            counterevidence_refs: epistemic.map(|value| value.counterevidence().to_vec()),
            checked_at: epistemic.map(|value| value.checked_at().to_owned()),
            expires_at: epistemic.map(|value| value.expires_at().to_owned()),
            refresh_trigger: epistemic.map(|value| value.refresh_trigger().as_str().to_owned()),
            decision_ref: epistemic.map(|value| value.decision().to_owned()),
            probe_ref: epistemic.map(|value| value.probe().to_owned()),
            falsifier_ref: epistemic.map(|value| value.falsifier().to_owned()),
        })
    }

    fn params(&self) -> [&(dyn ToSql + Sync); 48] {
        [
            &self.writer_project_id,
            &self.writer_project_snapshot_id,
            &self.writer_task_id,
            &self.writer_task_revision,
            &self.writer_task_spec_digest,
            &self.writer_attempt_id,
            &self.writer_lease_id,
            &self.writer_lease_holder_id,
            &self.writer_worktree_id,
            &self.writer_holder_process_id,
            &self.writer_holder_process_start_identity,
            &self.writer_daemon_instance_id,
            &self.writer_daemon_epoch,
            &self.writer_fencing_token,
            &self.writer_receipt_digest,
            &self.stream_id,
            &self.event_sequence,
            &self.event_digest,
            &self.command_id,
            &self.request_digest,
            &self.record_schema,
            &self.payload_schema,
            &self.payload_digest,
            &self.worker_id,
            &self.thread_id,
            &self.task_id,
            &self.branch_ref,
            &self.worktree_ref,
            &self.head_sha1,
            &self.foreman_state,
            &self.blocker_ref,
            &self.heartbeat_digest_ref,
            &self.authority_digest_ref,
            &self.evidence_digest_ref,
            &self.generation,
            &self.epistemic_schema,
            &self.observed_fact_refs,
            &self.hypothesis_refs,
            &self.confidence,
            &self.unknown_refs,
            &self.evidence_refs,
            &self.counterevidence_refs,
            &self.checked_at,
            &self.expires_at,
            &self.refresh_trigger,
            &self.decision_ref,
            &self.probe_ref,
            &self.falsifier_ref,
        ]
    }
}

struct StoreSqlValues {
    version: i16,
    transaction_id: String,
    project_id: String,
    project_snapshot_id: String,
    repository_owner: String,
    aggregate_key_digest: Vec<u8>,
    request_digest: Vec<u8>,
    authority_runtime: String,
    daemon_instance_id: String,
    daemon_epoch: i64,
    admission_mode: String,
    authority_revision: i64,
    authority_observation_digest: Vec<u8>,
    authority_head_digest: Vec<u8>,
    expected_head_runtime: String,
    expected_revision: i64,
    expected_state_digest: Vec<u8>,
    expected_head_digest: Vec<u8>,
    domain_command_digest: Vec<u8>,
    record_set_digest: Vec<u8>,
    next_state_digest: Vec<u8>,
    domain_receipt_digest: Vec<u8>,
    checkpoint_digest: Option<Vec<u8>>,
    outbox_intent_digest: Option<Vec<u8>>,
    genesis_state_digest: Vec<u8>,
    genesis_head_digest: Vec<u8>,
}

impl StoreSqlValues {
    fn new(
        request: &StoreTransactionRequest,
        canonical_request_digest: &ContentDigest,
        genesis: &StorePhysicalHead,
    ) -> PostgresTaskLedgerResult<Self> {
        Ok(Self {
            version: i16::try_from(request.version())
                .map_err(|_| error(PostgresTaskLedgerErrorKind::Malformed))?,
            transaction_id: request.transaction_id().as_str().to_owned(),
            project_id: request.scope().project_id().as_str().to_owned(),
            project_snapshot_id: request.scope().project_snapshot_id().as_str().to_owned(),
            repository_owner: request.scope().owner().as_str().to_owned(),
            aggregate_key_digest: digest_bytes(request.scope().aggregate_key_digest())?,
            request_digest: digest_bytes(canonical_request_digest)?,
            authority_runtime: "LIVE".to_owned(),
            daemon_instance_id: request
                .expected_authority()
                .daemon_instance_id()
                .as_str()
                .to_owned(),
            daemon_epoch: signed_i64(request.expected_authority().daemon_epoch().get())?,
            admission_mode: request.expected_authority().admission().as_str().to_owned(),
            authority_revision: signed_i64(request.expected_authority().revision().get())?,
            authority_observation_digest: digest_bytes(
                request.expected_authority().observation_digest(),
            )?,
            authority_head_digest: digest_bytes(request.expected_authority().head_digest())?,
            expected_head_runtime: "LIVE".to_owned(),
            expected_revision: signed_i64(request.expected_head().revision().get())?,
            expected_state_digest: digest_bytes(request.expected_head().state_digest())?,
            expected_head_digest: digest_bytes(request.expected_head().head_digest())?,
            domain_command_digest: digest_bytes(request.mutation().domain_command_digest())?,
            record_set_digest: digest_bytes(request.mutation().record_set_digest())?,
            next_state_digest: digest_bytes(request.mutation().next_state_digest())?,
            domain_receipt_digest: digest_bytes(request.mutation().domain_receipt_digest())?,
            checkpoint_digest: request
                .mutation()
                .checkpoint_digest()
                .map(digest_bytes)
                .transpose()?,
            outbox_intent_digest: request
                .mutation()
                .outbox_intent_digest()
                .map(digest_bytes)
                .transpose()?,
            genesis_state_digest: digest_bytes(genesis.state_digest())?,
            genesis_head_digest: digest_bytes(genesis.head_digest())?,
        })
    }

    fn params(&self) -> [&(dyn ToSql + Sync); 26] {
        [
            &self.version,
            &self.transaction_id,
            &self.project_id,
            &self.project_snapshot_id,
            &self.repository_owner,
            &self.aggregate_key_digest,
            &self.request_digest,
            &self.authority_runtime,
            &self.daemon_instance_id,
            &self.daemon_epoch,
            &self.admission_mode,
            &self.authority_revision,
            &self.authority_observation_digest,
            &self.authority_head_digest,
            &self.expected_head_runtime,
            &self.expected_revision,
            &self.expected_state_digest,
            &self.expected_head_digest,
            &self.domain_command_digest,
            &self.record_set_digest,
            &self.next_state_digest,
            &self.domain_receipt_digest,
            &self.checkpoint_digest,
            &self.outbox_intent_digest,
            &self.genesis_state_digest,
            &self.genesis_head_digest,
        ]
    }
}

struct StorePrepareRow {
    status: String,
    database_uuid: String,
    database_identity_digest: Option<Vec<u8>>,
    schema_version: i16,
    manifest_sha256: String,
    head_found: bool,
    before_revision: i64,
    before_state_digest: Vec<u8>,
    before_head_digest: Vec<u8>,
    after_revision: Option<i64>,
    after_state_digest: Option<Vec<u8>>,
    after_head_digest: Option<Vec<u8>>,
    disposition: Option<String>,
    transaction_digest: Option<Vec<u8>>,
    receipt_digest: Option<Vec<u8>>,
    global_schema_version: i16,
    global_manifest_sha256: String,
}

fn parse_store_prepare_row(row: &Row) -> PostgresTaskLedgerResult<StorePrepareRow> {
    if row.len() != 17 {
        return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt));
    }
    Ok(StorePrepareRow {
        status: row_value(row, 0)?,
        database_uuid: row_value(row, 1)?,
        database_identity_digest: row_value(row, 2)?,
        schema_version: row_value(row, 3)?,
        manifest_sha256: row_value(row, 4)?,
        head_found: row_value(row, 5)?,
        before_revision: row_value(row, 6)?,
        before_state_digest: row_value(row, 7)?,
        before_head_digest: row_value(row, 8)?,
        after_revision: row_value(row, 9)?,
        after_state_digest: row_value(row, 10)?,
        after_head_digest: row_value(row, 11)?,
        disposition: row_value(row, 12)?,
        transaction_digest: row_value(row, 13)?,
        receipt_digest: row_value(row, 14)?,
        global_schema_version: row_value(row, 15)?,
        global_manifest_sha256: row_value(row, 16)?,
    })
}

fn build_new_store_receipt(
    prepared: &StorePrepareRow,
    request: &StoreTransactionRequest,
    canonical_request_digest: &ContentDigest,
    loaded_physical_head: &StorePhysicalHead,
    database_uuid: &str,
    persistence: &StorePersistenceEvidence,
    global_persistence: &PostgresTaskLedgerPersistenceEvidence,
) -> PostgresTaskLedgerResult<StoreTransactionReceipt> {
    if prepared.status != "PREPARED"
        || prepared.database_uuid != database_uuid
        || prepared.schema_version
            != i16::try_from(persistence.schema_version()).unwrap_or_default()
        || prepared.manifest_sha256 != FROZEN_STORE_MANIFEST_SHA256
        || !global_persistence_matches(
            prepared.global_schema_version,
            &prepared.global_manifest_sha256,
            global_persistence,
        )
        || prepared.database_identity_digest.is_some()
        || prepared.after_revision.is_some()
        || prepared.after_state_digest.is_some()
        || prepared.after_head_digest.is_some()
        || prepared.disposition.is_some()
        || prepared.transaction_digest.is_some()
        || prepared.receipt_digest.is_some()
    {
        return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt));
    }
    let before = stored_physical_head(
        request.scope(),
        prepared.before_revision,
        &prepared.before_state_digest,
        &prepared.before_head_digest,
    )?;
    if before != *loaded_physical_head || before != *request.expected_head() {
        return Err(error(PostgresTaskLedgerErrorKind::PhysicalStateMismatch));
    }
    if prepared.head_found != (before.revision().get() != 0) {
        return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt));
    }
    let next_revision = before
        .revision()
        .get()
        .checked_add(1)
        .and_then(|revision| StoreRevision::new(revision).ok())
        .ok_or_else(|| error(PostgresTaskLedgerErrorKind::RevisionOverflow))?;
    let after = physical_head(
        RuntimeKind::Live,
        request.scope().clone(),
        next_revision,
        request.mutation().next_state_digest().clone(),
    )
    .map_err(|_| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))?;
    build_live_receipt(
        request.clone(),
        persistence.clone(),
        canonical_request_digest.clone(),
        before,
        after,
        StoreReceiptDisposition::Applied,
    )
    .map_err(|_| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))
}

enum StoreCallFailure {
    Database(PostgresError),
    Invalid(PostgresTaskLedgerError),
}

#[allow(clippy::too_many_arguments)]
fn call_store_finalize(
    transaction: &mut Transaction<'_>,
    values: &StoreSqlValues,
    database_uuid: &str,
    persistence: &StorePersistenceEvidence,
    sql_profile: TaskLedgerSqlProfile,
    global_schema_version: i16,
    global_manifest_sha256: &String,
    receipt: &StoreTransactionReceipt,
) -> Result<String, StoreCallFailure> {
    let database_identity_digest =
        digest_bytes(persistence.database_identity_digest()).map_err(StoreCallFailure::Invalid)?;
    let schema_version = i16::try_from(persistence.schema_version()).map_err(|_| {
        StoreCallFailure::Invalid(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))
    })?;
    let before_revision =
        signed_i64(receipt.before_head().revision().get()).map_err(StoreCallFailure::Invalid)?;
    let before_state_digest =
        digest_bytes(receipt.before_head().state_digest()).map_err(StoreCallFailure::Invalid)?;
    let before_head_digest =
        digest_bytes(receipt.before_head().head_digest()).map_err(StoreCallFailure::Invalid)?;
    let after_revision =
        signed_i64(receipt.after_head().revision().get()).map_err(StoreCallFailure::Invalid)?;
    let after_state_digest =
        digest_bytes(receipt.after_head().state_digest()).map_err(StoreCallFailure::Invalid)?;
    let after_head_digest =
        digest_bytes(receipt.after_head().head_digest()).map_err(StoreCallFailure::Invalid)?;
    let disposition = receipt.disposition().as_str();
    let transaction_digest =
        digest_bytes(receipt.transaction_digest()).map_err(StoreCallFailure::Invalid)?;
    let receipt_digest =
        digest_bytes(receipt.receipt_digest()).map_err(StoreCallFailure::Invalid)?;
    let params = global_profile_params(
        sql_profile,
        &global_schema_version,
        global_manifest_sha256,
        [
            &values.version,
            &values.transaction_id,
            &values.project_id,
            &values.project_snapshot_id,
            &values.repository_owner,
            &values.aggregate_key_digest,
            &values.request_digest,
            &values.authority_runtime,
            &values.daemon_instance_id,
            &values.daemon_epoch,
            &values.admission_mode,
            &values.authority_revision,
            &values.authority_observation_digest,
            &values.authority_head_digest,
            &values.expected_head_runtime,
            &values.expected_revision,
            &values.expected_state_digest,
            &values.expected_head_digest,
            &values.domain_command_digest,
            &values.record_set_digest,
            &values.next_state_digest,
            &values.domain_receipt_digest,
            &values.checkpoint_digest,
            &values.outbox_intent_digest,
            &values.genesis_state_digest,
            &values.genesis_head_digest,
            &database_uuid,
            &database_identity_digest,
            &schema_version,
            &FROZEN_STORE_MANIFEST_SHA256,
            &before_revision,
            &before_state_digest,
            &before_head_digest,
            &after_revision,
            &after_state_digest,
            &after_head_digest,
            &disposition,
            &transaction_digest,
            &receipt_digest,
        ],
    );
    let row = transaction
        .query_one(sql_profile.store_finalize_sql(), &params)
        .map_err(StoreCallFailure::Database)?;
    row.try_get(0).map_err(StoreCallFailure::Database)
}

fn global_profile_params<'a, const N: usize>(
    sql_profile: TaskLedgerSqlProfile,
    schema_version: &'a i16,
    manifest_sha256: &'a String,
    tail: [&'a (dyn ToSql + Sync); N],
) -> Vec<&'a (dyn ToSql + Sync)> {
    let mut params =
        Vec::with_capacity(N + usize::from(sql_profile.has_global_profile_parameters()) * 2);
    if sql_profile.has_global_profile_parameters() {
        params.push(schema_version as &(dyn ToSql + Sync));
        params.push(manifest_sha256 as &(dyn ToSql + Sync));
    }
    params.extend(tail);
    params
}

fn signed_i64(value: u64) -> PostgresTaskLedgerResult<i64> {
    i64::try_from(value).map_err(|_| error(PostgresTaskLedgerErrorKind::RevisionOverflow))
}

struct GeneralLedgerFinalizeValues {
    stream_id: Vec<u8>,
    project_id: String,
    project_snapshot_id: String,
    task_id: String,
    task_revision: String,
    task_subject_kind: String,
    task_subject_digest: Vec<u8>,
    next_sequence: String,
    next_last_event_digest: Vec<u8>,
    next_resource_revision: String,
    next_resource_projection_digest: Vec<u8>,
    next_head_digest: Vec<u8>,
    next_active_agents: String,
    next_active_implementers: String,
    next_elapsed_seconds: String,
    next_attempt_number: String,
    next_used_model_calls: String,
    next_used_external_cost: String,
    next_event_count: String,
    next_command_count: String,
    next_outbox_count: String,
    base_checkpoint_digest: Vec<u8>,
    next_checkpoint_digest: Vec<u8>,
    command_id: String,
    request_digest: Vec<u8>,
    expected_sequence: String,
    expected_last_event_digest: Vec<u8>,
    expected_resource_revision: String,
    expected_resource_projection_digest: Vec<u8>,
    expected_head_digest: Vec<u8>,
    correlation_id: String,
    occurred_at: String,
    actor_id: String,
    event_subject_digest: Vec<u8>,
    before_sequence: String,
    before_last_event_digest: Vec<u8>,
    before_resource_revision: String,
    before_resource_projection_digest: Vec<u8>,
    before_head_digest: Vec<u8>,
    after_sequence: String,
    after_last_event_digest: Vec<u8>,
    after_resource_revision: String,
    after_resource_projection_digest: Vec<u8>,
    after_head_digest: Vec<u8>,
    event_digest: Vec<u8>,
    receipt_digest: Vec<u8>,
    record_set_digest: Vec<u8>,
    store_transaction_id: String,
    event_sequence: String,
    previous_event_digest: Vec<u8>,
    event_resource_revision: String,
    event_resource_projection_digest: Vec<u8>,
}

impl GeneralLedgerFinalizeValues {
    #[allow(clippy::too_many_lines)]
    fn new(
        plan: &LedgerAppendPlan,
        store_receipt: &StoreTransactionReceipt,
    ) -> PostgresTaskLedgerResult<Self> {
        if plan.is_exact_retry() || plan.new_outbox().is_some() {
            return Err(error(PostgresTaskLedgerErrorKind::Malformed));
        }
        let command = plan
            .new_command()
            .ok_or_else(|| error(PostgresTaskLedgerErrorKind::Malformed))?
            .to_untrusted();
        let event = plan
            .new_event()
            .ok_or_else(|| error(PostgresTaskLedgerErrorKind::Malformed))?
            .to_untrusted();
        let next = plan.next_state();
        let identity = next.identity();
        let head = next.head();
        let counters = next.counters();
        let intake_digest = identity
            .general_task_intake_digest()
            .ok_or_else(|| error(PostgresTaskLedgerErrorKind::Malformed))?;

        if identity.subject_kind() != TaskLedgerSubjectKind::GeneralTaskIntake
            || identity.task_spec_digest().is_some()
            || identity.accounting_currency().is_some()
            || next.events().len() != 1
            || next.commands().len() != 1
            || !next.outboxes().is_empty()
            || head.sequence() != 1
            || head.resource_revision() != 0
            || counters.active_agents() != 0
            || counters.active_implementers() != 0
            || counters.elapsed_seconds() != 0
            || counters.attempt_number() != 0
            || counters.used_model_calls() != 0
            || counters.used_external_cost() != "0"
            || command.request.kind != LedgerEventKind::TaskCreated.as_str()
            || command.request.action != TaskCreatedProfile::GeneralTaskIntakeV1.action()
            || command.request.outcome != "RECORDED"
            || command.request.reason_code != "GENERAL_TASK_INTAKE_RECORDED"
            || command.request.diagnostic.is_some()
            || command.request.resource_snapshot.is_some()
            || command.receipt.outcome != "APPENDED"
            || command.receipt.denial_reason.is_some()
            || command.receipt.event_digest.as_ref() != Some(&event.event_digest)
            || event.kind != LedgerEventKind::TaskCreated.as_str()
            || event.action != TaskCreatedProfile::GeneralTaskIntakeV1.action()
            || event.outcome != "RECORDED"
            || event.reason_code != "GENERAL_TASK_INTAKE_RECORDED"
            || event.diagnostic.is_some()
            || event.resource_snapshot.is_some()
            || event.sequence != 1
            || event.resource_revision != 0
            || event.subject_digest != command.request.subject_digest
            || event.command_id != command.command_id
            || event.request_digest != command.receipt.request_digest
            || &event.stream_identity != identity
            || event.stream_id != *head.stream_id()
            || command.request.expected_head.identity() != identity
            || command.request.expected_head.stream_id() != head.stream_id()
            || command.receipt.before != command.request.expected_head
            || command.receipt.after != *head
        {
            return Err(error(PostgresTaskLedgerErrorKind::Malformed));
        }

        Ok(Self {
            stream_id: digest_bytes(head.stream_id())?,
            project_id: identity.project_id().as_str().to_owned(),
            project_snapshot_id: identity.project_snapshot_id().as_str().to_owned(),
            task_id: identity.task_id().as_str().to_owned(),
            task_revision: identity.task_revision().to_owned(),
            task_subject_kind: identity.subject_kind().as_str().to_owned(),
            task_subject_digest: digest_bytes(intake_digest)?,
            next_sequence: head.sequence().to_string(),
            next_last_event_digest: digest_bytes(head.last_event_digest())?,
            next_resource_revision: head.resource_revision().to_string(),
            next_resource_projection_digest: digest_bytes(head.resource_projection_digest())?,
            next_head_digest: digest_bytes(head.head_digest())?,
            next_active_agents: counters.active_agents().to_string(),
            next_active_implementers: counters.active_implementers().to_string(),
            next_elapsed_seconds: counters.elapsed_seconds().to_string(),
            next_attempt_number: counters.attempt_number().to_string(),
            next_used_model_calls: counters.used_model_calls().to_string(),
            next_used_external_cost: counters.used_external_cost().to_owned(),
            next_event_count: next.events().len().to_string(),
            next_command_count: next.commands().len().to_string(),
            next_outbox_count: next.outboxes().len().to_string(),
            base_checkpoint_digest: digest_bytes(plan.base_checkpoint().checkpoint_digest())?,
            next_checkpoint_digest: digest_bytes(plan.next_checkpoint().checkpoint_digest())?,
            command_id: command.command_id,
            request_digest: digest_bytes(&command.receipt.request_digest)?,
            expected_sequence: command.request.expected_head.sequence().to_string(),
            expected_last_event_digest: digest_bytes(
                command.request.expected_head.last_event_digest(),
            )?,
            expected_resource_revision: command
                .request
                .expected_head
                .resource_revision()
                .to_string(),
            expected_resource_projection_digest: digest_bytes(
                command.request.expected_head.resource_projection_digest(),
            )?,
            expected_head_digest: digest_bytes(command.request.expected_head.head_digest())?,
            correlation_id: command.request.correlation_id,
            occurred_at: command.request.occurred_at,
            actor_id: command.request.actor_id,
            event_subject_digest: digest_bytes(&command.request.subject_digest)?,
            before_sequence: command.receipt.before.sequence().to_string(),
            before_last_event_digest: digest_bytes(command.receipt.before.last_event_digest())?,
            before_resource_revision: command.receipt.before.resource_revision().to_string(),
            before_resource_projection_digest: digest_bytes(
                command.receipt.before.resource_projection_digest(),
            )?,
            before_head_digest: digest_bytes(command.receipt.before.head_digest())?,
            after_sequence: command.receipt.after.sequence().to_string(),
            after_last_event_digest: digest_bytes(command.receipt.after.last_event_digest())?,
            after_resource_revision: command.receipt.after.resource_revision().to_string(),
            after_resource_projection_digest: digest_bytes(
                command.receipt.after.resource_projection_digest(),
            )?,
            after_head_digest: digest_bytes(command.receipt.after.head_digest())?,
            event_digest: digest_bytes(&event.event_digest)?,
            receipt_digest: digest_bytes(&command.receipt.receipt_digest)?,
            record_set_digest: digest_bytes(plan.record_set_digest())?,
            store_transaction_id: store_receipt.request().transaction_id().as_str().to_owned(),
            event_sequence: event.sequence.to_string(),
            previous_event_digest: digest_bytes(&event.previous_event_digest)?,
            event_resource_revision: event.resource_revision.to_string(),
            event_resource_projection_digest: digest_bytes(&event.resource_projection_digest)?,
        })
    }

    fn params(&self) -> [&(dyn ToSql + Sync); 52] {
        [
            &self.stream_id,
            &self.project_id,
            &self.project_snapshot_id,
            &self.task_id,
            &self.task_revision,
            &self.task_subject_kind,
            &self.task_subject_digest,
            &self.next_sequence,
            &self.next_last_event_digest,
            &self.next_resource_revision,
            &self.next_resource_projection_digest,
            &self.next_head_digest,
            &self.next_active_agents,
            &self.next_active_implementers,
            &self.next_elapsed_seconds,
            &self.next_attempt_number,
            &self.next_used_model_calls,
            &self.next_used_external_cost,
            &self.next_event_count,
            &self.next_command_count,
            &self.next_outbox_count,
            &self.base_checkpoint_digest,
            &self.next_checkpoint_digest,
            &self.command_id,
            &self.request_digest,
            &self.expected_sequence,
            &self.expected_last_event_digest,
            &self.expected_resource_revision,
            &self.expected_resource_projection_digest,
            &self.expected_head_digest,
            &self.correlation_id,
            &self.occurred_at,
            &self.actor_id,
            &self.event_subject_digest,
            &self.before_sequence,
            &self.before_last_event_digest,
            &self.before_resource_revision,
            &self.before_resource_projection_digest,
            &self.before_head_digest,
            &self.after_sequence,
            &self.after_last_event_digest,
            &self.after_resource_revision,
            &self.after_resource_projection_digest,
            &self.after_head_digest,
            &self.event_digest,
            &self.receipt_digest,
            &self.record_set_digest,
            &self.store_transaction_id,
            &self.event_sequence,
            &self.previous_event_digest,
            &self.event_resource_revision,
            &self.event_resource_projection_digest,
        ]
    }
}

struct LedgerFinalizeValues {
    stream_id: Vec<u8>,
    project_id: String,
    project_snapshot_id: String,
    task_id: String,
    task_revision: String,
    task_spec_digest: Vec<u8>,
    accounting_currency: String,
    next_sequence: String,
    next_last_event_digest: Vec<u8>,
    next_resource_revision: String,
    next_resource_projection_digest: Vec<u8>,
    next_head_digest: Vec<u8>,
    next_active_agents: String,
    next_active_implementers: String,
    next_elapsed_seconds: String,
    next_attempt_number: String,
    next_used_model_calls: String,
    next_used_external_cost: String,
    next_event_count: String,
    next_command_count: String,
    next_outbox_count: String,
    base_checkpoint_digest: Vec<u8>,
    next_checkpoint_digest: Vec<u8>,
    command_id: String,
    request_digest: Vec<u8>,
    expected_sequence: String,
    expected_last_event_digest: Vec<u8>,
    expected_resource_revision: String,
    expected_resource_projection_digest: Vec<u8>,
    expected_head_digest: Vec<u8>,
    correlation_id: String,
    occurred_at: String,
    event_kind: String,
    actor_id: String,
    action_id: String,
    audit_outcome: String,
    reason_code: String,
    subject_digest: Vec<u8>,
    diagnostic: JsonValue,
    has_resource_snapshot: bool,
    resource_active_agents: String,
    resource_active_implementers: String,
    resource_elapsed_seconds: String,
    resource_attempt_number: String,
    resource_used_model_calls: String,
    resource_used_external_cost: String,
    before_sequence: String,
    before_last_event_digest: Vec<u8>,
    before_resource_revision: String,
    before_resource_projection_digest: Vec<u8>,
    before_head_digest: Vec<u8>,
    after_sequence: String,
    after_last_event_digest: Vec<u8>,
    after_resource_revision: String,
    after_resource_projection_digest: Vec<u8>,
    after_head_digest: Vec<u8>,
    command_outcome: String,
    denial_reason: String,
    event_digest: Vec<u8>,
    receipt_digest: Vec<u8>,
    record_set_digest: Vec<u8>,
    store_transaction_id: String,
    append_event: bool,
    event_sequence: String,
    previous_event_digest: Vec<u8>,
    event_resource_revision: String,
    event_resource_projection_digest: Vec<u8>,
    admit_outbox: bool,
    admission_digest: Vec<u8>,
    intent_digest: Vec<u8>,
}

impl LedgerFinalizeValues {
    #[allow(clippy::too_many_lines)]
    fn new(
        plan: &LedgerAppendPlan,
        store_receipt: &StoreTransactionReceipt,
    ) -> PostgresTaskLedgerResult<Self> {
        let command = plan
            .new_command()
            .ok_or_else(|| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))?
            .to_untrusted();
        let next = plan.next_state();
        let identity = next.identity();
        let head = next.head();
        let counters = next.counters();
        let zero = vec![0_u8; 32];
        let (has_resource_snapshot, resource) = match command.request.resource_snapshot.as_ref() {
            Some(counters) => (true, counters.clone()),
            None => (
                false,
                ResourceCounters::new(0, 0, 0, 0, 0, "0")
                    .map_err(|_| error(PostgresTaskLedgerErrorKind::Malformed))?,
            ),
        };
        let diagnostic = match command.request.diagnostic.as_ref() {
            None => JsonValue::Null,
            Some(CanonicalValue::Null) => {
                return Err(error(PostgresTaskLedgerErrorKind::Malformed));
            }
            Some(value) => canonical_to_json(value),
        };
        let event = plan
            .new_event()
            .map(lattice_task_ledger::LedgerEvent::to_untrusted);
        let outbox = plan
            .new_outbox()
            .map(lattice_task_ledger::OutboxAdmission::to_untrusted);
        let event_digest = command
            .receipt
            .event_digest
            .as_ref()
            .map(digest_bytes)
            .transpose()?
            .unwrap_or_else(|| zero.clone());
        let denial_reason = command.receipt.denial_reason.clone().unwrap_or_default();
        let event_sequence = event
            .as_ref()
            .map_or_else(|| "0".to_owned(), |event| event.sequence.to_string());
        let previous_event_digest = event
            .as_ref()
            .map(|event| digest_bytes(&event.previous_event_digest))
            .transpose()?
            .unwrap_or_else(|| zero.clone());
        let event_resource_revision = event.as_ref().map_or_else(
            || "0".to_owned(),
            |event| event.resource_revision.to_string(),
        );
        let event_resource_projection_digest = event
            .as_ref()
            .map(|event| digest_bytes(&event.resource_projection_digest))
            .transpose()?
            .unwrap_or_else(|| zero.clone());
        let admission_digest = outbox
            .as_ref()
            .map(|outbox| digest_bytes(&outbox.admission_digest))
            .transpose()?
            .unwrap_or_else(|| zero.clone());
        let intent_digest = outbox
            .as_ref()
            .map(|outbox| digest_bytes(&outbox.intent_digest))
            .transpose()?
            .unwrap_or_else(|| zero.clone());
        let task_spec_digest = identity
            .task_spec_digest()
            .ok_or_else(|| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))?;
        let accounting_currency = identity
            .accounting_currency()
            .ok_or_else(|| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))?;
        Ok(Self {
            stream_id: digest_bytes(head.stream_id())?,
            project_id: identity.project_id().as_str().to_owned(),
            project_snapshot_id: identity.project_snapshot_id().as_str().to_owned(),
            task_id: identity.task_id().as_str().to_owned(),
            task_revision: identity.task_revision().to_owned(),
            task_spec_digest: digest_bytes(task_spec_digest)?,
            accounting_currency: accounting_currency.to_owned(),
            next_sequence: head.sequence().to_string(),
            next_last_event_digest: digest_bytes(head.last_event_digest())?,
            next_resource_revision: head.resource_revision().to_string(),
            next_resource_projection_digest: digest_bytes(head.resource_projection_digest())?,
            next_head_digest: digest_bytes(head.head_digest())?,
            next_active_agents: counters.active_agents().to_string(),
            next_active_implementers: counters.active_implementers().to_string(),
            next_elapsed_seconds: counters.elapsed_seconds().to_string(),
            next_attempt_number: counters.attempt_number().to_string(),
            next_used_model_calls: counters.used_model_calls().to_string(),
            next_used_external_cost: counters.used_external_cost().to_owned(),
            next_event_count: next.events().len().to_string(),
            next_command_count: next.commands().len().to_string(),
            next_outbox_count: next.outboxes().len().to_string(),
            base_checkpoint_digest: digest_bytes(plan.base_checkpoint().checkpoint_digest())?,
            next_checkpoint_digest: digest_bytes(plan.next_checkpoint().checkpoint_digest())?,
            command_id: command.command_id,
            request_digest: digest_bytes(&command.receipt.request_digest)?,
            expected_sequence: command.request.expected_head.sequence().to_string(),
            expected_last_event_digest: digest_bytes(
                command.request.expected_head.last_event_digest(),
            )?,
            expected_resource_revision: command
                .request
                .expected_head
                .resource_revision()
                .to_string(),
            expected_resource_projection_digest: digest_bytes(
                command.request.expected_head.resource_projection_digest(),
            )?,
            expected_head_digest: digest_bytes(command.request.expected_head.head_digest())?,
            correlation_id: command.request.correlation_id,
            occurred_at: command.request.occurred_at,
            event_kind: command.request.kind,
            actor_id: command.request.actor_id,
            action_id: command.request.action,
            audit_outcome: command.request.outcome,
            reason_code: command.request.reason_code,
            subject_digest: digest_bytes(&command.request.subject_digest)?,
            diagnostic,
            has_resource_snapshot,
            resource_active_agents: resource.active_agents().to_string(),
            resource_active_implementers: resource.active_implementers().to_string(),
            resource_elapsed_seconds: resource.elapsed_seconds().to_string(),
            resource_attempt_number: resource.attempt_number().to_string(),
            resource_used_model_calls: resource.used_model_calls().to_string(),
            resource_used_external_cost: resource.used_external_cost().to_owned(),
            before_sequence: command.receipt.before.sequence().to_string(),
            before_last_event_digest: digest_bytes(command.receipt.before.last_event_digest())?,
            before_resource_revision: command.receipt.before.resource_revision().to_string(),
            before_resource_projection_digest: digest_bytes(
                command.receipt.before.resource_projection_digest(),
            )?,
            before_head_digest: digest_bytes(command.receipt.before.head_digest())?,
            after_sequence: command.receipt.after.sequence().to_string(),
            after_last_event_digest: digest_bytes(command.receipt.after.last_event_digest())?,
            after_resource_revision: command.receipt.after.resource_revision().to_string(),
            after_resource_projection_digest: digest_bytes(
                command.receipt.after.resource_projection_digest(),
            )?,
            after_head_digest: digest_bytes(command.receipt.after.head_digest())?,
            command_outcome: command.receipt.outcome,
            denial_reason,
            event_digest,
            receipt_digest: digest_bytes(&command.receipt.receipt_digest)?,
            record_set_digest: digest_bytes(plan.record_set_digest())?,
            store_transaction_id: store_receipt.request().transaction_id().as_str().to_owned(),
            append_event: event.is_some(),
            event_sequence,
            previous_event_digest,
            event_resource_revision,
            event_resource_projection_digest,
            admit_outbox: outbox.is_some(),
            admission_digest,
            intent_digest,
        })
    }

    fn params(&self) -> [&(dyn ToSql + Sync); 70] {
        [
            &self.stream_id,
            &self.project_id,
            &self.project_snapshot_id,
            &self.task_id,
            &self.task_revision,
            &self.task_spec_digest,
            &self.accounting_currency,
            &self.next_sequence,
            &self.next_last_event_digest,
            &self.next_resource_revision,
            &self.next_resource_projection_digest,
            &self.next_head_digest,
            &self.next_active_agents,
            &self.next_active_implementers,
            &self.next_elapsed_seconds,
            &self.next_attempt_number,
            &self.next_used_model_calls,
            &self.next_used_external_cost,
            &self.next_event_count,
            &self.next_command_count,
            &self.next_outbox_count,
            &self.base_checkpoint_digest,
            &self.next_checkpoint_digest,
            &self.command_id,
            &self.request_digest,
            &self.expected_sequence,
            &self.expected_last_event_digest,
            &self.expected_resource_revision,
            &self.expected_resource_projection_digest,
            &self.expected_head_digest,
            &self.correlation_id,
            &self.occurred_at,
            &self.event_kind,
            &self.actor_id,
            &self.action_id,
            &self.audit_outcome,
            &self.reason_code,
            &self.subject_digest,
            &self.diagnostic,
            &self.has_resource_snapshot,
            &self.resource_active_agents,
            &self.resource_active_implementers,
            &self.resource_elapsed_seconds,
            &self.resource_attempt_number,
            &self.resource_used_model_calls,
            &self.resource_used_external_cost,
            &self.before_sequence,
            &self.before_last_event_digest,
            &self.before_resource_revision,
            &self.before_resource_projection_digest,
            &self.before_head_digest,
            &self.after_sequence,
            &self.after_last_event_digest,
            &self.after_resource_revision,
            &self.after_resource_projection_digest,
            &self.after_head_digest,
            &self.command_outcome,
            &self.denial_reason,
            &self.event_digest,
            &self.receipt_digest,
            &self.record_set_digest,
            &self.store_transaction_id,
            &self.append_event,
            &self.event_sequence,
            &self.previous_event_digest,
            &self.event_resource_revision,
            &self.event_resource_projection_digest,
            &self.admit_outbox,
            &self.admission_digest,
            &self.intent_digest,
        ]
    }
}

fn canonical_to_json(value: &CanonicalValue) -> JsonValue {
    match value {
        CanonicalValue::Null => JsonValue::Null,
        CanonicalValue::Bool(value) => JsonValue::Bool(*value),
        CanonicalValue::String(value) => JsonValue::String(value.clone()),
        CanonicalValue::Array(values) => {
            JsonValue::Array(values.iter().map(canonical_to_json).collect())
        }
        CanonicalValue::Object(entries) => JsonValue::Object(
            entries
                .iter()
                .map(|(key, value)| (key.clone(), canonical_to_json(value)))
                .collect(),
        ),
    }
}

fn store_request_for_plan(
    plan: &LedgerAppendPlan,
    expected_authority: StoreAuthorityHead,
    expected_physical_head: StorePhysicalHead,
    outbox: Option<&OutboxAdmission>,
) -> PostgresTaskLedgerResult<StoreTransactionRequest> {
    let command = plan.command_record().request();
    let identity = command.expected_head().identity();
    let scope = StoreScope::new(
        identity.project_id().clone(),
        identity.project_snapshot_id().clone(),
        StoreRepositoryOwner::TaskLedger,
        command.expected_head().stream_id().clone(),
    )
    .map_err(|_| error(PostgresTaskLedgerErrorKind::Malformed))?;
    if expected_physical_head.scope() != &scope {
        return Err(error(PostgresTaskLedgerErrorKind::PhysicalStateMismatch));
    }
    let checkpoint = plan
        .command_record()
        .result_checkpoint()
        .checkpoint_digest()
        .clone();
    let mutation = StoreMutationCommitment::new(
        plan.receipt().request_digest().clone(),
        plan.record_set_digest().clone(),
        checkpoint.clone(),
        plan.receipt().receipt_digest().clone(),
        Some(checkpoint),
        outbox.map(|admission| admission.admission_digest().clone()),
    )
    .map_err(|_| error(PostgresTaskLedgerErrorKind::Malformed))?;
    let transaction_id = store_transaction_id(
        command.expected_head().stream_id(),
        command.command_id().as_str(),
    )?;
    StoreTransactionRequest::new(
        STORE_CONTRACT_VERSION,
        transaction_id,
        scope,
        expected_authority,
        expected_physical_head,
        mutation,
    )
    .map_err(|_| error(PostgresTaskLedgerErrorKind::Malformed))
}

fn store_transaction_id(
    stream_id: &ContentDigest,
    command_id: &str,
) -> PostgresTaskLedgerResult<StoreTransactionId> {
    let domain = HashDomain::new(STORE_TRANSACTION_ID_SCHEMA, STORE_TRANSACTION_ID_VERSION)
        .map_err(|_| error(PostgresTaskLedgerErrorKind::Malformed))?;
    let subject = object([
        (
            "repository_owner",
            CanonicalValue::String(StoreRepositoryOwner::TaskLedger.as_str().to_owned()),
        ),
        (
            "stream_id",
            CanonicalValue::String(stream_id.as_str().to_owned()),
        ),
        ("command_id", CanonicalValue::String(command_id.to_owned())),
    ]);
    let hash = canonical_sha256(&domain, &subject)
        .map_err(|_| error(PostgresTaskLedgerErrorKind::Malformed))?;
    StoreTransactionId::new(format!("{STORE_TRANSACTION_ID_PREFIX}{}", hash.to_hex()))
        .map_err(|_| error(PostgresTaskLedgerErrorKind::Malformed))
}

fn parse_u64_text(value: &str) -> PostgresTaskLedgerResult<u64> {
    let parsed = value
        .parse::<u64>()
        .map_err(|_| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))?;
    if parsed.to_string() != value {
        return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt));
    }
    Ok(parsed)
}

fn diagnostic_from_json(value: &JsonValue) -> PostgresTaskLedgerResult<Diagnostic> {
    let canonical = json_to_canonical(value)?;
    let diagnostic = Diagnostic::new(canonical.clone())
        .map_err(|_| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))?;
    if diagnostic.value() != &canonical {
        return Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt));
    }
    Ok(diagnostic)
}

fn json_to_canonical(value: &JsonValue) -> PostgresTaskLedgerResult<CanonicalValue> {
    match value {
        JsonValue::Null => Ok(CanonicalValue::Null),
        JsonValue::Bool(value) => Ok(CanonicalValue::Bool(*value)),
        JsonValue::String(value) => Ok(CanonicalValue::String(value.clone())),
        JsonValue::Array(values) => values
            .iter()
            .map(json_to_canonical)
            .collect::<PostgresTaskLedgerResult<Vec<_>>>()
            .map(CanonicalValue::Array),
        JsonValue::Object(values) => values
            .iter()
            .map(|(key, value)| Ok((key.clone(), json_to_canonical(value)?)))
            .collect::<PostgresTaskLedgerResult<Vec<_>>>()
            .map(CanonicalValue::Object),
        JsonValue::Number(_) => Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt)),
    }
}

fn retryable_sqlstate(code: &str, constraint: Option<&str>) -> bool {
    matches!(code, "40001" | "40P01")
        || (code == "23505"
            && matches!(
                constraint,
                Some("task_ledger_streams_pkey" | "task_ingress_claims_pkey")
            ))
}

fn map_ledger_error(value: &LedgerError) -> PostgresTaskLedgerError {
    let kind = match value {
        LedgerError::CommandIdReuse => PostgresTaskLedgerErrorKind::CommandSubstitution,
        LedgerError::CheckpointMismatch => PostgresTaskLedgerErrorKind::CheckpointCorrupt,
        LedgerError::UnknownForemanSnapshotVersion => {
            PostgresTaskLedgerErrorKind::UnsupportedRetainedSchema
        }
        _ => PostgresTaskLedgerErrorKind::RetainedRowCorrupt,
    };
    error(kind)
}

fn map_plan_error(value: &LedgerError) -> PostgresTaskLedgerError {
    let kind = match value {
        LedgerError::CommandIdReuse => PostgresTaskLedgerErrorKind::CommandSubstitution,
        LedgerError::CheckpointMismatch => PostgresTaskLedgerErrorKind::CheckpointCorrupt,
        _ => PostgresTaskLedgerErrorKind::Malformed,
    };
    error(kind)
}

fn map_setup_error(value: crate::PostgresStoreSetupError) -> PostgresTaskLedgerError {
    let kind = match value.kind() {
        PostgresStoreSetupErrorKind::NetworkBoundary => PostgresTaskLedgerErrorKind::Unavailable,
        PostgresStoreSetupErrorKind::TransactionFailed => {
            PostgresTaskLedgerErrorKind::TransactionFailed
        }
        PostgresStoreSetupErrorKind::CommitOutcomeUnknown => {
            PostgresTaskLedgerErrorKind::CommitOutcomeUnknown
        }
        PostgresStoreSetupErrorKind::TargetMismatch => PostgresTaskLedgerErrorKind::Malformed,
        PostgresStoreSetupErrorKind::UnsupportedFutureSchema => {
            PostgresTaskLedgerErrorKind::UnsupportedRetainedSchema
        }
        _ => PostgresTaskLedgerErrorKind::RetainedRowCorrupt,
    };
    error(kind)
}

fn digest(value: &str) -> PostgresTaskLedgerResult<ContentDigest> {
    ContentDigest::from_sha256(value.to_owned())
        .map_err(|_| error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))
}

fn object<const N: usize>(entries: [(&str, CanonicalValue); N]) -> CanonicalValue {
    CanonicalValue::Object(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

const fn error(kind: PostgresTaskLedgerErrorKind) -> PostgresTaskLedgerError {
    PostgresTaskLedgerError::new(kind)
}

#[cfg(test)]
mod tests {
    use lattice_contracts::{
        ContentDigest, ProjectId, ProjectSnapshotId, RuntimeAdmissionMode, RuntimeKind,
        StoreAuthorityHead, StoreAuthorityRevision, StoreDaemonInstanceId, StoreRevision, TaskId,
        TaskLedgerStreamIdentity,
    };
    use lattice_task_ledger::{
        ActionId, ActorId, AppendCommand, AutonomyAppendMetadata, AutonomyAuthorityEvidence,
        AutonomyDecisionReason, AutonomyIntent, AutonomyObservedTaskState, AutonomyRecommendation,
        AutonomyRiskClass, AutonomyTaskKind, CommandId, CorrelationId, LedgerEventKind,
        LedgerOutcome, ReasonCode, VerifiedStream, apply_append_plan, plan_append,
        plan_autonomy_receipt_append,
    };
    use serde_json::json;

    use super::*;
    use crate::genesis_head;

    fn fixed_digest(byte: char) -> ContentDigest {
        ContentDigest::from_sha256(byte.to_string().repeat(64)).expect("digest")
    }

    #[test]
    fn setup_mapping_reserves_unsupported_for_coherent_future_schema_only() {
        assert_eq!(
            map_setup_error(crate::PostgresStoreSetupError::new(
                PostgresStoreSetupErrorKind::UnsupportedFutureSchema,
            ))
            .kind(),
            PostgresTaskLedgerErrorKind::UnsupportedRetainedSchema
        );
        for kind in [
            PostgresStoreSetupErrorKind::HistoryMismatch,
            PostgresStoreSetupErrorKind::CompatibilityMismatch,
            PostgresStoreSetupErrorKind::CorruptCatalog,
        ] {
            assert_eq!(
                map_setup_error(crate::PostgresStoreSetupError::new(kind)).kind(),
                PostgresTaskLedgerErrorKind::RetainedRowCorrupt
            );
        }
    }

    #[test]
    fn neutral_ingress_claim_action_closure_preserves_historical_canary_only() {
        for action in [
            "CONTROLLED_CODEX_CANARY",
            "CONTROLLED_CODEX_CANARY_AUTONOMY_V1",
        ] {
            assert!(ingress_claim_event_action_matches(
                TaskIngressRequestKind::ControlledCodexCanary,
                Some(action)
            ));
            assert!(!ingress_claim_event_action_matches(
                TaskIngressRequestKind::GeneralTask,
                Some(action)
            ));
        }
        assert!(ingress_claim_event_action_matches(
            TaskIngressRequestKind::GeneralTask,
            Some("GENERAL_TASK_INTAKE_V1")
        ));
        for action in [
            "CONTROLLED_CODEX_CANARY_V2",
            "GENERAL_TASK_INTAKE_V2",
            "TASK_CREATED",
        ] {
            assert!(!ingress_claim_event_action_matches(
                TaskIngressRequestKind::ControlledCodexCanary,
                Some(action)
            ));
            assert!(!ingress_claim_event_action_matches(
                TaskIngressRequestKind::GeneralTask,
                Some(action)
            ));
        }
    }

    #[test]
    fn neutral_ingress_claim_requires_exact_command_client_binding() {
        let canary = TaskIngressClaim::controlled_canary(
            "lattice_task_submit.v1",
            "request-1",
            fixed_digest('1'),
        )
        .expect("canary claim");
        assert!(ingress_claim_command_matches(
            &canary,
            "mcp-submit:request-1"
        ));
        for command_id in [
            "request-1",
            "mcp-submit:request-2",
            "MCP-SUBMIT:request-1",
            "mcp-submit:request-1:extra",
        ] {
            assert!(!ingress_claim_command_matches(&canary, command_id));
        }
    }

    #[test]
    fn submission_lookup_uses_the_shared_secret_free_client_request_contract() {
        assert!(valid_submission_lookup_id("lattice_task_submit.v1"));
        assert!(valid_task_ingress_client_request_id(
            "general-task-request-1"
        ));
        for rejected in [
            "secret:value",
            "ghp_abcdefghijklmnopqrstuvwxyz123456",
            "prefix-sk-do-not-use",
            "AKIA1234567890ABCDEF",
        ] {
            assert!(valid_submission_lookup_id(rejected));
            assert!(!valid_task_ingress_client_request_id(rejected));
        }
    }

    fn identity(project: &str) -> TaskLedgerStreamIdentity {
        TaskLedgerStreamIdentity::new(
            ProjectId::new(project).expect("project"),
            ProjectSnapshotId::new(format!("{project}:snapshot:1")).expect("snapshot"),
            TaskId::new("TASK-021").expect("task"),
            "1",
            fixed_digest('a'),
            "TWD",
        )
        .expect("identity")
    }

    fn command(stream: &VerifiedStream, command_id: &str) -> AppendCommand {
        AppendCommand::new(
            stream.head().clone(),
            CommandId::new(command_id).expect("command id"),
            CorrelationId::new("correlation-1").expect("correlation"),
            "2026-08-02T00:00:00Z",
            LedgerEventKind::TaskCreated,
            ActorId::new("lattice-pm").expect("actor"),
            ActionId::new("record-task").expect("action"),
            LedgerOutcome::Recorded,
            ReasonCode::new("TASK_ACCEPTED").expect("reason"),
            fixed_digest('b'),
            None,
            None,
        )
        .expect("command")
    }

    fn authority() -> StoreAuthorityHead {
        authority_with_head('d')
    }

    fn authority_with_head(byte: char) -> StoreAuthorityHead {
        StoreAuthorityHead::new(
            RuntimeKind::Live,
            StoreDaemonInstanceId::new("daemon-1").expect("daemon"),
            lattice_contracts::DaemonEpoch::new(1).expect("epoch"),
            RuntimeAdmissionMode::Active,
            StoreAuthorityRevision::new(1).expect("revision"),
            fixed_digest('c'),
            fixed_digest(byte),
        )
        .expect("authority")
    }

    fn ask_user_autonomy_plan(
        expected_store_authority: &StoreAuthorityHead,
    ) -> AutonomyReceiptAppendPlan {
        let vacant =
            VerifiedStream::vacant(identity("project-1"), RuntimeKind::Live).expect("stream");
        let created_plan = plan_append(
            &vacant,
            AppendCommand::new_autonomy_required_task_created(
                vacant.head().clone(),
                CommandId::new("required-profile").expect("command"),
                CorrelationId::new("correlation-1").expect("correlation"),
                "2026-08-13T00:00:00Z",
                ActorId::new("lattice-runtime").expect("actor"),
                ReasonCode::new("TASK038_TASK_ACCEPTED").expect("reason"),
                fixed_digest('9'),
                None,
            )
            .expect("required command"),
        )
        .expect("required plan");
        let created = apply_append_plan(&vacant, &created_plan).expect("created stream");
        plan_autonomy_receipt_append(
            &created,
            AutonomyAppendMetadata::new(
                CommandId::new("autonomy").expect("command"),
                CorrelationId::new("correlation-1").expect("correlation"),
                "2026-08-13T00:00:01Z",
                ActorId::new("lattice-runtime").expect("actor"),
            )
            .expect("metadata"),
            AutonomyIntent::new(
                AutonomyTaskKind::Feature,
                AutonomyRiskClass::R0,
                false,
                false,
                false,
                AutonomyObservedTaskState::Draft,
                AutonomyRecommendation::AskUser {
                    reason: AutonomyDecisionReason::NewUserDecision,
                },
            ),
            AutonomyAuthorityEvidence::new_p0_process_start_profile(
                fixed_digest('1'),
                fixed_digest('2'),
                expected_store_authority.head_digest().clone(),
                None,
            )
            .expect("authority evidence"),
        )
        .expect("autonomy plan")
    }

    #[test]
    fn autonomy_plan_binds_the_transaction_store_authority_head() {
        let expected = authority();
        let plan = ask_user_autonomy_plan(&expected);
        assert!(autonomy_plan_matches_store_authority(&plan, &expected));
        assert!(!autonomy_plan_matches_store_authority(
            &plan,
            &authority_with_head('e')
        ));
    }

    #[test]
    fn transaction_identity_is_deterministic_and_cross_stream_safe() {
        let first =
            VerifiedStream::vacant(identity("project-1"), RuntimeKind::Live).expect("stream");
        let same = store_transaction_id(first.head().stream_id(), "command-1").expect("id");
        let repeat = store_transaction_id(first.head().stream_id(), "command-1").expect("id");
        let other_command =
            store_transaction_id(first.head().stream_id(), "command-2").expect("id");
        let other_stream =
            VerifiedStream::vacant(identity("project-2"), RuntimeKind::Live).expect("stream");
        let cross_stream =
            store_transaction_id(other_stream.head().stream_id(), "command-1").expect("id");

        assert_eq!(same, repeat);
        assert_eq!(same.as_str().len(), 79);
        assert!(same.as_str().starts_with(STORE_TRANSACTION_ID_PREFIX));
        assert_ne!(same, other_command);
        assert_ne!(same, cross_stream);
    }

    #[test]
    fn store_scope_and_mutation_mapping_are_exhaustive() {
        let stream =
            VerifiedStream::vacant(identity("project-1"), RuntimeKind::Live).expect("stream");
        let plan = plan_append(&stream, command(&stream, "command-1")).expect("plan");
        let scope = StoreScope::new(
            stream.identity().project_id().clone(),
            stream.identity().project_snapshot_id().clone(),
            StoreRepositoryOwner::TaskLedger,
            stream.head().stream_id().clone(),
        )
        .expect("scope");
        let physical = genesis_head(RuntimeKind::Live, scope).expect("genesis");
        let request = store_request_for_plan(&plan, authority(), physical, plan.new_outbox())
            .expect("request");

        assert_eq!(request.scope().owner(), StoreRepositoryOwner::TaskLedger);
        assert_eq!(
            request.scope().aggregate_key_digest(),
            stream.head().stream_id()
        );
        assert_eq!(
            request.mutation().domain_command_digest(),
            plan.receipt().request_digest()
        );
        assert_eq!(
            request.mutation().record_set_digest(),
            plan.record_set_digest()
        );
        assert_eq!(
            request.mutation().next_state_digest(),
            plan.command_record()
                .result_checkpoint()
                .checkpoint_digest()
        );
        assert_eq!(
            request.mutation().domain_receipt_digest(),
            plan.receipt().receipt_digest()
        );
        assert_eq!(
            request.mutation().checkpoint_digest(),
            Some(
                plan.command_record()
                    .result_checkpoint()
                    .checkpoint_digest()
            )
        );
        assert_eq!(request.mutation().outbox_intent_digest(), None);
    }

    #[test]
    fn canonical_u64_boundary_rejects_narrowing_and_aliases() {
        assert_eq!(parse_u64_text("0"), Ok(0));
        assert_eq!(parse_u64_text("18446744073709551615"), Ok(u64::MAX));
        for invalid in ["", "+1", "01", "-1", "1.0", "18446744073709551616"] {
            assert_eq!(
                parse_u64_text(invalid),
                Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))
            );
        }
    }

    #[test]
    fn physical_revision_must_equal_retained_and_actual_command_counts() {
        let stream =
            VerifiedStream::vacant(identity("project-1"), RuntimeKind::Live).expect("stream");
        let scope = StoreScope::new(
            stream.identity().project_id().clone(),
            stream.identity().project_snapshot_id().clone(),
            StoreRepositoryOwner::TaskLedger,
            stream.head().stream_id().clone(),
        )
        .expect("scope");
        let physical = physical_head(
            RuntimeKind::Live,
            scope,
            StoreRevision::new(2).expect("revision"),
            fixed_digest('e'),
        )
        .expect("physical head");

        assert_eq!(validate_physical_command_count(&physical, 2, 2), Ok(()));
        for (retained, actual) in [(1, 2), (2, 1), (3, 3)] {
            assert_eq!(
                validate_physical_command_count(&physical, retained, actual),
                Err(error(PostgresTaskLedgerErrorKind::PhysicalStateMismatch))
            );
        }
    }

    #[test]
    fn diagnostic_json_rejects_numbers_before_domain_revalidation() {
        let accepted = diagnostic_from_json(&json!({"ok": true, "nested": [null, "text"]}))
            .expect("diagnostic");
        assert!(matches!(accepted.value(), CanonicalValue::Object(_)));
        assert_eq!(
            diagnostic_from_json(&json!({"number": 1})),
            Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))
        );
    }

    #[test]
    fn retry_classification_is_exact_and_poison_is_fail_closed() {
        assert!(retryable_sqlstate("40001", None));
        assert!(retryable_sqlstate("40P01", None));
        assert!(retryable_sqlstate(
            "23505",
            Some("task_ledger_streams_pkey")
        ));
        assert!(retryable_sqlstate(
            "23505",
            Some("task_ingress_claims_pkey")
        ));
        assert!(!retryable_sqlstate("23505", Some("other_unique")));
        assert!(!retryable_sqlstate("23505", Some("physical_heads_pkey")));
        assert!(!retryable_sqlstate(
            "23505",
            Some("terminal_transactions_pkey")
        ));
        assert!(!retryable_sqlstate("08006", None));

        assert_eq!(
            commit_failure_class(None, None),
            CommitFailureClass::OutcomeUnknown
        );
        assert_eq!(
            commit_failure_class(Some("40001"), None),
            CommitFailureClass::Retryable
        );
        assert_eq!(
            commit_failure_class(Some("23505"), Some("other_unique")),
            CommitFailureClass::Terminal
        );
        assert_eq!(
            commit_failure_class(Some("55P03"), None),
            CommitFailureClass::Terminal
        );
    }

    #[test]
    fn runtime_transactions_have_fixed_timeouts_and_queries_append_global_identity() {
        for settings in [WRITE_TRANSACTION_SETTINGS, READ_TRANSACTION_SETTINGS] {
            assert!(settings.contains("SET LOCAL lock_timeout = '5s'"));
            assert!(settings.contains("SET LOCAL statement_timeout = '30s'"));
        }
        assert!(
            LEDGER_HEAD_V5_SQL
                .contains("physical_head_digest, global_schema_version, global_manifest_sha256")
        );
        assert!(LEDGER_HEAD_V3_SQL.contains("task_ledger_read_head_v1("));
        assert!(LEDGER_HEAD_V5_SQL.contains("task_ledger_read_head_v3("));
        assert!(
            STORE_PREPARE_V5_SQL
                .contains("terminal_receipt_digest, global_schema_version, global_manifest_sha256")
        );
        assert!(
            STORE_CURRENT_V5_SQL
                .contains("head_digest, global_schema_version, global_manifest_sha256")
        );
        assert!(STORE_PREPARE_V3_SQL.contains("schema_version, manifest_sha256, head_found"));
    }

    #[test]
    fn global_persistence_comparison_is_exact_and_distinct_from_store_profile() {
        let global = PostgresTaskLedgerPersistenceEvidence {
            database_identity_digest: fixed_digest('e'),
            schema_version: CURRENT_GLOBAL_LEDGER_SCHEMA_VERSION,
            manifest_digest: fixed_digest('f'),
        };
        assert!(global_persistence_matches(
            5,
            global.manifest_digest().as_str(),
            &global
        ));
        assert!(!global_persistence_matches(
            2,
            global.manifest_digest().as_str(),
            &global
        ));
        assert!(!global_persistence_matches(
            3,
            FROZEN_STORE_MANIFEST_SHA256,
            &global
        ));
        assert_eq!(FROZEN_STORE_SCHEMA_VERSION, 2);
    }

    #[test]
    fn task_ledger_routes_only_verified_historical_and_submission_profiles() {
        assert_eq!(
            global_ledger_sql_profile(LEGACY_GLOBAL_LEDGER_SCHEMA_VERSION),
            Some(TaskLedgerSqlProfile::V3)
        );
        assert_eq!(
            global_ledger_sql_profile(CURRENT_GLOBAL_LEDGER_SCHEMA_VERSION),
            Some(TaskLedgerSqlProfile::V5)
        );
        assert_eq!(
            global_ledger_sql_profile(FOREMAN_GLOBAL_LEDGER_SCHEMA_VERSION),
            Some(TaskLedgerSqlProfile::V6)
        );
        assert_eq!(
            global_ledger_sql_profile(SUBMISSION_GLOBAL_LEDGER_SCHEMA_VERSION),
            Some(TaskLedgerSqlProfile::V7)
        );
        assert!(TaskLedgerSqlProfile::V6.supports_foreman());
        assert!(TaskLedgerSqlProfile::V7.supports_foreman());
        assert!(TaskLedgerSqlProfile::V7.supports_submission());
        assert!(!TaskLedgerSqlProfile::V6.supports_submission());
        assert!(!TaskLedgerSqlProfile::V5.supports_foreman());
        for version in [0, 1, 2, 4, u16::MAX] {
            assert_eq!(global_ledger_sql_profile(version), None);
        }
        assert_eq!(
            LEGACY_GLOBAL_LEDGER_MANIFEST_SHA256,
            "09c431df18ad71a4f44239a5d2ddf6b1774b8ffec06c7f9223f0e41757f3d407"
        );
    }

    #[test]
    fn historical_v3_profile_never_routes_to_v5_autonomy_functions() {
        let historical = TaskLedgerSqlProfile::V3;
        assert!(!historical.supports_autonomy());
        assert_eq!(historical.autonomy_receipts_sql(), None);
        assert_eq!(historical.autonomy_record_sql(), None);
        assert!(!historical.ledger_events_sql().contains("autonomy"));

        let current = TaskLedgerSqlProfile::V5;
        assert_eq!(
            current.autonomy_receipts_sql(),
            Some(LEDGER_AUTONOMY_RECEIPTS_SQL)
        );
        assert_eq!(
            current.autonomy_record_sql(),
            Some(LEDGER_RECORD_AUTONOMY_RECEIPT_SQL)
        );
    }

    #[test]
    fn historical_v3_profile_rejects_new_required_task_profile() {
        let stream =
            VerifiedStream::vacant(identity("project-1"), RuntimeKind::Live).expect("stream");
        let plan = plan_append(
            &stream,
            AppendCommand::new_autonomy_required_task_created(
                stream.head().clone(),
                CommandId::new("required-profile").expect("command"),
                CorrelationId::new("correlation-1").expect("correlation"),
                "2026-08-13T00:00:00Z",
                ActorId::new("lattice-runtime").expect("actor"),
                ReasonCode::new("TASK038_TASK_ACCEPTED").expect("reason"),
                fixed_digest('9'),
                None,
            )
            .expect("required command"),
        )
        .expect("required plan");
        assert!(plan_uses_autonomy_surface(&plan));
        assert!(!TaskLedgerSqlProfile::V3.supports_autonomy());
    }

    #[test]
    fn historical_v3_load_rejects_retained_autonomy_surface() {
        let vacant =
            VerifiedStream::vacant(identity("project-1"), RuntimeKind::Live).expect("stream");
        let created_plan = plan_append(
            &vacant,
            AppendCommand::new_autonomy_required_task_created(
                vacant.head().clone(),
                CommandId::new("required-profile").expect("command"),
                CorrelationId::new("correlation-1").expect("correlation"),
                "2026-08-13T00:00:00Z",
                ActorId::new("lattice-runtime").expect("actor"),
                ReasonCode::new("TASK038_TASK_ACCEPTED").expect("reason"),
                fixed_digest('9'),
                None,
            )
            .expect("required command"),
        )
        .expect("required plan");
        let pending = apply_append_plan(&vacant, &created_plan).expect("pending stream");
        assert_eq!(
            validate_autonomy_surface_for_profile(&pending, TaskLedgerSqlProfile::V3),
            Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))
        );

        let receipt_plan = ask_user_autonomy_plan(&authority());
        let complete =
            apply_append_plan(&pending, receipt_plan.append_plan()).expect("complete stream");
        assert!(
            complete
                .events()
                .iter()
                .any(|event| { event.kind() == LedgerEventKind::AutonomyReceiptRecorded })
        );
        assert_eq!(
            validate_autonomy_surface_for_profile(&complete, TaskLedgerSqlProfile::V3),
            Err(error(PostgresTaskLedgerErrorKind::RetainedRowCorrupt))
        );
    }

    #[test]
    fn timeout_sqlstates_are_terminal_unavailable() {
        for code in ["55P03", "57014"] {
            assert_eq!(
                database_error_kind(code),
                PostgresTaskLedgerErrorKind::Unavailable
            );
        }
    }

    #[test]
    fn foreman_payload_rejection_is_malformed_not_a_retryable_transaction_failure() {
        assert_eq!(
            database_error_kind("LFW01"),
            PostgresTaskLedgerErrorKind::Malformed
        );
    }

    #[test]
    fn ledger_error_mapping_preserves_substitution_and_checkpoint_classes() {
        assert_eq!(
            map_ledger_error(&LedgerError::CommandIdReuse).kind(),
            PostgresTaskLedgerErrorKind::CommandSubstitution
        );
        assert_eq!(
            map_ledger_error(&LedgerError::CheckpointMismatch).kind(),
            PostgresTaskLedgerErrorKind::CheckpointCorrupt
        );
    }

    #[test]
    fn foreman_snapshot_query_preserves_select_from_token_boundary() {
        assert!(LEDGER_FOREMAN_SNAPSHOTS_SQL.contains("falsifier_ref FROM "));
        assert!(!LEDGER_FOREMAN_SNAPSHOTS_SQL.contains("falsifier_refFROM"));
    }
}
