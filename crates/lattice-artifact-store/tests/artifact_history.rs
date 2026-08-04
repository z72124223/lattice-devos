#[allow(dead_code)]
#[path = "../src/history.rs"]
mod history;

use std::cell::Cell;
use std::collections::HashSet;

use history::{
    ArtifactCommandExecutionDisposition, ArtifactCommandHistory, ArtifactCommandKind,
    ArtifactCommandObjectScope, ArtifactCommandOutcome, ArtifactCommandRequest,
    ArtifactCommandStorageKey, ArtifactCommandTerminalProjection, ArtifactHashDomain,
    ArtifactHistoryCheckpoint, ArtifactHistoryError,
};
use lattice_cjson::CanonicalValue;
use lattice_contracts::{ArtifactCounter, ContentDigest, ProjectId};

fn digest(hex: char) -> ContentDigest {
    ContentDigest::from_sha256(hex.to_string().repeat(64)).expect("valid digest")
}

fn key(project: &str, object_hex: char, command_id: &str) -> ArtifactCommandStorageKey {
    ArtifactCommandStorageKey::new(
        ProjectId::new(project).expect("project"),
        digest(object_hex),
        command_id,
    )
    .expect("storage key")
}

fn scope(project: &str, object_hex: char) -> ArtifactCommandObjectScope {
    ArtifactCommandObjectScope::new(
        ProjectId::new(project).expect("project"),
        digest(object_hex),
    )
}

fn source(name: &str) -> CanonicalValue {
    CanonicalValue::Object(vec![
        (
            "request_id".to_owned(),
            CanonicalValue::String(name.to_owned()),
        ),
        (
            "expected_revision".to_owned(),
            CanonicalValue::String("7".to_owned()),
        ),
    ])
}

fn request(
    project: &str,
    object_hex: char,
    command_id: &str,
    kind: ArtifactCommandKind,
    name: &str,
) -> ArtifactCommandRequest {
    ArtifactCommandRequest::new(key(project, object_hex, command_id), kind, source(name))
        .expect("request")
}

fn applied() -> ArtifactCommandTerminalProjection {
    ArtifactCommandTerminalProjection::applied(digest('b'), digest('c'), digest('d'))
        .expect("applied projection")
}

fn denied() -> ArtifactCommandTerminalProjection {
    ArtifactCommandTerminalProjection::denied(
        "REFERENCE_LIMIT",
        digest('c'),
        digest('c'),
        digest('e'),
    )
    .expect("denied projection")
}

fn object_field_mut<'a>(value: &'a mut CanonicalValue, field: &str) -> &'a mut CanonicalValue {
    let CanonicalValue::Object(fields) = value else {
        panic!("expected object")
    };
    fields
        .iter_mut()
        .find_map(|(name, value)| (name == field).then_some(value))
        .expect("field")
}

fn object_fields_mut(value: &mut CanonicalValue) -> &mut Vec<(String, CanonicalValue)> {
    let CanonicalValue::Object(fields) = value else {
        panic!("expected object")
    };
    fields
}

fn records_mut(value: &mut CanonicalValue) -> &mut Vec<CanonicalValue> {
    let CanonicalValue::Array(records) = object_field_mut(value, "records") else {
        panic!("expected records")
    };
    records
}

fn set_string(value: &mut CanonicalValue, replacement: &str) {
    let CanonicalValue::String(current) = value else {
        panic!("expected string")
    };
    replacement.clone_into(current);
}

#[test]
fn command_storage_key_is_exactly_project_algorithm_object_and_command() {
    let key = ArtifactCommandStorageKey::new(
        ProjectId::new("project-a").expect("project"),
        digest('a'),
        "command-1",
    )
    .expect("storage key");

    assert_eq!(key.project_id().as_str(), "project-a");
    assert_eq!(key.algorithm(), "sha256");
    assert_eq!(key.content_digest(), &digest('a'));
    assert_eq!(key.command_id(), "command-1");
    assert_eq!(ArtifactCommandKind::Publish.as_str(), "PUBLISH");
    assert!(
        ArtifactCommandStorageKey::new(
            ProjectId::new("project-a").expect("project"),
            digest('a'),
            "path/not-allowed",
        )
        .is_err()
    );
}

#[test]
fn command_kind_set_is_closed_and_covers_every_frozen_lifecycle_command() {
    let kinds = ArtifactCommandKind::ALL
        .into_iter()
        .map(ArtifactCommandKind::as_str)
        .collect::<HashSet<_>>();

    assert_eq!(kinds.len(), 11);
    for required in [
        "PUBLISH",
        "ADD_REFERENCE",
        "RELEASE_REFERENCE",
        "ACQUIRE_READ",
        "RELEASE_READ",
        "EXPIRE_READ",
        "RECONCILE_READ",
        "STAGING",
        "DELETE_CLAIM",
        "DELETE_RESULT",
        "DELETE_RECONCILE",
    ] {
        assert!(kinds.contains(required), "missing {required}");
    }
}

#[test]
fn sanitized_request_source_rejects_untyped_or_oversized_metadata() {
    let storage_key = key("project-a", 'a', "command-1");
    let not_an_object = ArtifactCommandRequest::new(
        storage_key.clone(),
        ArtifactCommandKind::Publish,
        CanonicalValue::String("not-an-object".to_owned()),
    )
    .expect_err("source must be an object");
    assert!(matches!(
        not_an_object,
        ArtifactHistoryError::InvalidRequestSource {
            field: "request_source"
        }
    ));

    let secret = "SECRET-MUST-NOT-ENTER-HISTORY";
    for (field, value) in [
        ("raw", "plaintext"),
        ("raw_bytes", "plaintext"),
        ("payload", "plaintext"),
        ("content", "plaintext"),
        ("body", "plaintext"),
        ("data", "plaintext"),
        ("note", secret),
        ("encoded_blob", "U0VDUkVULVJBVy1CWVRFUw=="),
    ] {
        let error = ArtifactCommandRequest::new(
            storage_key.clone(),
            ArtifactCommandKind::Publish,
            CanonicalValue::Object(vec![(
                field.to_owned(),
                CanonicalValue::String(value.to_owned()),
            )]),
        )
        .expect_err("untyped or plaintext field");
        assert!(matches!(error, ArtifactHistoryError::ForbiddenRequestField));
        assert!(!format!("{error:?}").contains(value));
    }

    let oversized_leaf = ArtifactCommandRequest::new(
        storage_key.clone(),
        ArtifactCommandKind::Publish,
        CanonicalValue::Object(vec![(
            "request_id".to_owned(),
            CanonicalValue::String("x".repeat(257)),
        )]),
    )
    .expect_err("bounded string leaf");
    assert!(matches!(
        oversized_leaf,
        ArtifactHistoryError::RequestSourceLimit {
            field: "string_leaf"
        }
    ));
}

#[test]
fn typed_safe_scalar_metadata_is_redacted_from_debug_and_contains_no_rejected_secret() {
    let storage_key = key("project-a", 'a', "command-1");
    let secret = "SECRET-MUST-NOT-ENTER-HISTORY";
    let safe = ArtifactCommandRequest::new(
        storage_key,
        ArtifactCommandKind::Publish,
        CanonicalValue::Object(vec![
            (
                "content_digest".to_owned(),
                CanonicalValue::String(digest('a').as_str().to_owned()),
            ),
            (
                "payload_digest".to_owned(),
                CanonicalValue::String(digest('b').as_str().to_owned()),
            ),
            (
                "byte_length".to_owned(),
                CanonicalValue::String("123".to_owned()),
            ),
            (
                "produced_at".to_owned(),
                CanonicalValue::String("2026-07-30T00:00:00Z".to_owned()),
            ),
            (
                "authority_token".to_owned(),
                CanonicalValue::String("authority-1".to_owned()),
            ),
        ]),
    )
    .expect("typed scalar metadata is allowed");
    let debug = format!("{safe:?}");
    assert!(debug.contains("[ELIDED]"));
    assert!(!debug.contains(secret));

    let mut history = ArtifactCommandHistory::new();
    let receipt = history
        .execute(safe, || Ok(applied()))
        .expect("recorded receipt");
    assert!(!format!("{receipt:?}").contains(secret));
    assert!(!format!("{history:?}").contains(secret));
    assert!(
        !receipt
            .receipt()
            .canonical_bytes()
            .expect("receipt bytes")
            .windows(secret.len())
            .any(|window| window == secret.as_bytes())
    );
    assert!(
        !format!(
            "{:?}",
            history
                .export_untrusted(&scope("project-a", 'a'))
                .expect("raw history")
        )
        .contains(secret)
    );
}

#[test]
fn denied_projection_cannot_claim_a_state_change() {
    let error = ArtifactCommandTerminalProjection::denied(
        "POLICY_DENIED",
        digest('b'),
        digest('c'),
        digest('d'),
    )
    .expect_err("denial must preserve state");

    assert_eq!(error, ArtifactHistoryError::DeniedStateChanged);
    assert_eq!(error.code(), "ARTIFACT_DENIAL_STATE_CHANGED");
}

#[test]
fn exact_retry_precedes_evaluation_and_returns_byte_identical_receipt() {
    let mut history = ArtifactCommandHistory::new();
    let first_request = request(
        "project-a",
        'a',
        "command-1",
        ArtifactCommandKind::Publish,
        "artifact",
    );
    let first = history
        .execute(first_request.clone(), || Ok(applied()))
        .expect("first execution");
    let calls = Cell::new(0_u8);
    let retry = history
        .execute(first_request, || {
            calls.set(calls.get() + 1);
            Err(ArtifactHistoryError::CheckpointMismatch)
        })
        .expect("exact retry");

    assert_eq!(calls.get(), 0, "retry must skip currentness and time");
    assert_eq!(
        retry.disposition(),
        ArtifactCommandExecutionDisposition::ExactRetry
    );
    assert_eq!(retry.receipt(), first.receipt());
    assert_eq!(
        retry.receipt().canonical_bytes().expect("retry bytes"),
        first.receipt().canonical_bytes().expect("first bytes")
    );
    assert_eq!(
        history
            .head(&scope("project-a", 'a'))
            .expect("head")
            .high_water()
            .get(),
        1
    );
}

#[test]
fn request_lookup_reports_exact_retry_reuse_or_vacancy_without_mutation() {
    let mut history = ArtifactCommandHistory::new();
    let first_request = request(
        "project-a",
        'a',
        "command-1",
        ArtifactCommandKind::Publish,
        "artifact",
    );
    let before = history.clone();
    assert_eq!(
        history
            .lookup_request(&first_request)
            .expect("vacant lookup"),
        None
    );
    assert_eq!(history, before);

    let recorded = history
        .execute(first_request.clone(), || Ok(applied()))
        .expect("record");
    let before = history.clone();
    let exact = history
        .lookup_request(&first_request)
        .expect("exact lookup")
        .expect("stored receipt");
    assert_eq!(exact, *recorded.receipt());
    assert_eq!(history, before);

    let changed = request(
        "project-a",
        'a',
        "command-1",
        ArtifactCommandKind::Publish,
        "changed",
    );
    assert_eq!(
        history
            .lookup_request(&changed)
            .expect_err("changed request"),
        ArtifactHistoryError::CommandIdReuse
    );
    assert_eq!(history, before);
}

#[test]
fn evaluator_error_leaves_command_history_exactly_unchanged() {
    let mut history = ArtifactCommandHistory::new();
    let request = request(
        "project-a",
        'a',
        "command-1",
        ArtifactCommandKind::Publish,
        "artifact",
    );
    let before = history.clone();
    let calls = Cell::new(0_u8);

    let error = history
        .execute(request.clone(), || {
            calls.set(calls.get() + 1);
            Err(ArtifactHistoryError::CheckpointMismatch)
        })
        .expect_err("evaluation error");

    assert_eq!(error, ArtifactHistoryError::CheckpointMismatch);
    assert_eq!(calls.get(), 1);
    assert_eq!(history, before, "an error must not leave an empty stream");
    assert_eq!(
        history
            .lookup_request(&request)
            .expect("lookup after error"),
        None
    );
}

#[test]
fn same_key_changed_full_source_is_permanent_reuse_without_high_water_change() {
    let mut history = ArtifactCommandHistory::new();
    history
        .execute(
            request(
                "project-a",
                'a',
                "command-1",
                ArtifactCommandKind::Publish,
                "first",
            ),
            || Ok(applied()),
        )
        .expect("first");
    let calls = Cell::new(0_u8);
    let error = history
        .execute(
            request(
                "project-a",
                'a',
                "command-1",
                ArtifactCommandKind::Publish,
                "changed",
            ),
            || {
                calls.set(calls.get() + 1);
                Ok(applied())
            },
        )
        .expect_err("changed source must fail");

    assert_eq!(error, ArtifactHistoryError::CommandIdReuse);
    assert_eq!(error.code(), "ARTIFACT_COMMAND_ID_REUSE");
    assert_eq!(calls.get(), 0);
    assert_eq!(
        history
            .head(&scope("project-a", 'a'))
            .expect("head")
            .high_water()
            .get(),
        1
    );
}

#[test]
fn canonical_field_order_is_the_same_full_request_but_kind_drift_is_reuse() {
    let mut history = ArtifactCommandHistory::new();
    let storage_key = key("project-a", 'a', "command-1");
    let first_source = CanonicalValue::Object(vec![
        (
            "task_id".to_owned(),
            CanonicalValue::String("task-last".to_owned()),
        ),
        (
            "request_id".to_owned(),
            CanonicalValue::String("request-first".to_owned()),
        ),
    ]);
    let reordered_source = CanonicalValue::Object(vec![
        (
            "request_id".to_owned(),
            CanonicalValue::String("request-first".to_owned()),
        ),
        (
            "task_id".to_owned(),
            CanonicalValue::String("task-last".to_owned()),
        ),
    ]);
    history
        .execute(
            ArtifactCommandRequest::new(
                storage_key.clone(),
                ArtifactCommandKind::Publish,
                first_source,
            )
            .expect("first request"),
            || Ok(applied()),
        )
        .expect("first");
    let retry = history
        .execute(
            ArtifactCommandRequest::new(
                storage_key.clone(),
                ArtifactCommandKind::Publish,
                reordered_source.clone(),
            )
            .expect("retry request"),
            || panic!("canonical retry must not evaluate"),
        )
        .expect("canonical retry");
    assert_eq!(
        retry.disposition(),
        ArtifactCommandExecutionDisposition::ExactRetry
    );

    let kind_drift = history
        .execute(
            ArtifactCommandRequest::new(
                storage_key,
                ArtifactCommandKind::AddReference,
                reordered_source,
            )
            .expect("kind-drift request"),
            || panic!("kind drift must not evaluate"),
        )
        .expect_err("kind drift is command-id reuse");
    assert_eq!(kind_drift, ArtifactHistoryError::CommandIdReuse);
}

#[test]
fn identical_command_ids_are_independent_across_project_and_object_scope() {
    let mut history = ArtifactCommandHistory::new();
    for (project, object_hex) in [("project-a", 'a'), ("project-a", 'b'), ("project-b", 'a')] {
        let execution = history
            .execute(
                request(
                    project,
                    object_hex,
                    "same-command",
                    ArtifactCommandKind::Publish,
                    "artifact",
                ),
                || Ok(applied()),
            )
            .expect("independent execution");
        assert_eq!(
            execution.disposition(),
            ArtifactCommandExecutionDisposition::Recorded
        );
        assert_eq!(
            history
                .head(&scope(project, object_hex))
                .expect("head")
                .high_water()
                .get(),
            1
        );
    }
}

#[test]
fn applied_and_denied_receipts_share_one_ordinal_and_predecessor_chain() {
    let mut history = ArtifactCommandHistory::new();
    let applied_receipt = history
        .execute(
            request(
                "project-a",
                'a',
                "command-1",
                ArtifactCommandKind::Publish,
                "artifact",
            ),
            || Ok(applied()),
        )
        .expect("applied");
    let denied_receipt = history
        .execute(
            request(
                "project-a",
                'a',
                "command-2",
                ArtifactCommandKind::AddReference,
                "second-reference",
            ),
            || Ok(denied()),
        )
        .expect("denied");

    assert_eq!(applied_receipt.receipt().ordinal().get(), 1);
    assert_eq!(
        applied_receipt.receipt().outcome(),
        ArtifactCommandOutcome::Applied
    );
    assert_eq!(
        applied_receipt.receipt().request().kind(),
        ArtifactCommandKind::Publish
    );
    assert_eq!(
        applied_receipt.receipt().request().source(),
        &source("artifact")
    );
    assert_eq!(
        applied_receipt
            .receipt()
            .request()
            .request_digest()
            .as_str()
            .len(),
        64
    );
    assert_eq!(applied_receipt.receipt().predecessor_digest(), None);
    assert_eq!(denied_receipt.receipt().ordinal().get(), 2);
    assert_eq!(
        denied_receipt.receipt().outcome(),
        ArtifactCommandOutcome::Denied
    );
    assert_eq!(
        denied_receipt.receipt().predecessor_digest(),
        Some(applied_receipt.receipt().receipt_digest())
    );
    assert_eq!(
        denied_receipt.receipt().denial_code(),
        Some("REFERENCE_LIMIT")
    );
    assert_eq!(denied_receipt.receipt().before_state_digest(), &digest('c'));
    assert_eq!(denied_receipt.receipt().after_state_digest(), &digest('c'));
    assert_eq!(denied_receipt.receipt().result_digest(), &digest('e'));
    assert_ne!(
        denied_receipt.receipt().record_digest(),
        denied_receipt.receipt().receipt_digest()
    );

    let head = history.head(&scope("project-a", 'a')).expect("head");
    assert_eq!(head.high_water().get(), 2);
    assert_eq!(head.denial_count().get(), 1);
    assert_eq!(head.head_digest().as_str().len(), 64);
    assert_eq!(
        head.denial_tail_digest(),
        Some(denied_receipt.receipt().receipt_digest())
    );
}

#[test]
fn raw_export_strictly_replays_against_an_independent_trusted_checkpoint() {
    let (history, raw, checkpoint) = two_record_history();
    let replayed =
        ArtifactCommandHistory::replay_untrusted(&raw, &checkpoint).expect("strict replay");

    assert_eq!(
        replayed
            .head(&scope("project-a", 'a'))
            .expect("replayed head"),
        history
            .head(&scope("project-a", 'a'))
            .expect("original head")
    );

    let head = history.head(&scope("project-a", 'a')).expect("head");
    let independently_constructed = ArtifactHistoryCheckpoint::new_trusted(
        head.scope().clone(),
        head.high_water(),
        head.tail_digest().cloned(),
        head.denial_count(),
        head.denial_tail_digest().cloned(),
    )
    .expect("independent checkpoint");
    assert_eq!(independently_constructed, checkpoint);
    assert_eq!(
        independently_constructed.head().head_digest(),
        head.head_digest()
    );
    assert_eq!(
        independently_constructed.checkpoint_digest().as_str().len(),
        64
    );
}

#[test]
fn strict_replay_rejects_unknown_version_kind_field_and_malformed_count() {
    let (_, raw, checkpoint) = two_record_history();

    let mut unknown_version = raw.clone();
    set_string(object_field_mut(&mut unknown_version, "version"), "2");
    assert_eq!(
        ArtifactCommandHistory::replay_untrusted(&unknown_version, &checkpoint)
            .expect_err("version"),
        ArtifactHistoryError::UnknownVersion
    );

    let mut unknown_kind = raw.clone();
    set_string(
        object_field_mut(&mut records_mut(&mut unknown_kind)[0], "kind"),
        "FUTURE_KIND",
    );
    let unknown_kind_error = ArtifactCommandHistory::replay_untrusted(&unknown_kind, &checkpoint)
        .expect_err("unknown kind");
    assert_eq!(unknown_kind_error, ArtifactHistoryError::UnknownKind);
    assert!(!format!("{unknown_kind_error:?}").contains("FUTURE_KIND"));

    let mut unknown_field = raw.clone();
    object_fields_mut(&mut unknown_field).push((
        "future".to_owned(),
        CanonicalValue::String("unsupported".to_owned()),
    ));
    let unknown_field_error = ArtifactCommandHistory::replay_untrusted(&unknown_field, &checkpoint)
        .expect_err("unknown field");
    assert_eq!(unknown_field_error, ArtifactHistoryError::UnknownField);
    assert!(!format!("{unknown_field_error:?}").contains("future"));

    let mut malformed = raw;
    set_string(
        object_field_mut(&mut records_mut(&mut malformed)[0], "ordinal"),
        "01",
    );
    assert!(matches!(
        ArtifactCommandHistory::replay_untrusted(&malformed, &checkpoint),
        Err(ArtifactHistoryError::Malformed { field: "ordinal" })
    ));
}

#[test]
fn replay_preflight_bounds_depth_strings_records_bytes_and_signed_bigint() {
    let (_, raw, checkpoint) = two_record_history();

    let mut too_deep = raw.clone();
    let mut nested = CanonicalValue::Null;
    for _ in 0..40 {
        nested = CanonicalValue::Array(vec![nested]);
    }
    object_fields_mut(&mut too_deep).push(("future".to_owned(), nested));
    assert!(matches!(
        ArtifactCommandHistory::replay_untrusted(&too_deep, &checkpoint),
        Err(ArtifactHistoryError::ReplayLimit { field: "depth" })
    ));

    let secret = "SENSITIVE-RAW-CONTENT";
    let mut oversized_string = raw.clone();
    object_fields_mut(&mut oversized_string).push((
        "future".to_owned(),
        CanonicalValue::String(secret.repeat(4_000)),
    ));
    let oversized_error = ArtifactCommandHistory::replay_untrusted(&oversized_string, &checkpoint)
        .expect_err("oversized raw string");
    assert!(matches!(
        oversized_error,
        ArtifactHistoryError::ReplayLimit {
            field: "string_leaf"
        }
    ));
    assert!(!format!("{oversized_error:?}").contains(secret));

    assert!(matches!(
        ArtifactCommandHistory::replay_untrusted_with_bounds(&raw, &checkpoint, 1, 1_073_741_824,),
        Err(ArtifactHistoryError::ReplayLimit {
            field: "record_count"
        })
    ));
    assert!(matches!(
        ArtifactCommandHistory::replay_untrusted_with_bounds(&raw, &checkpoint, 1_000_000, 64),
        Err(ArtifactHistoryError::ReplayLimit {
            field: "canonical_bytes"
        })
    ));

    let mut oversized_counter = raw;
    let raw_head = object_field_mut(&mut oversized_counter, "head");
    set_string(
        object_field_mut(raw_head, "high_water"),
        &(i64::MAX as u64 + 1).to_string(),
    );
    assert!(matches!(
        ArtifactCommandHistory::replay_untrusted(&oversized_counter, &checkpoint),
        Err(ArtifactHistoryError::Malformed {
            field: "high_water"
        })
    ));
}

#[test]
fn strict_replay_rejects_tamper_reorder_truncation_and_duplicate_command() {
    let (_, raw, checkpoint) = two_record_history();

    let mut denied_state_change = raw.clone();
    set_string(
        object_field_mut(
            &mut records_mut(&mut denied_state_change)[1],
            "after_state_digest",
        ),
        digest('f').as_str(),
    );
    assert_eq!(
        ArtifactCommandHistory::replay_untrusted(&denied_state_change, &checkpoint)
            .expect_err("denied state change"),
        ArtifactHistoryError::DeniedStateChanged
    );

    let mut tampered = raw.clone();
    set_string(
        object_field_mut(&mut records_mut(&mut tampered)[0], "result_digest"),
        digest('f').as_str(),
    );
    assert_eq!(
        ArtifactCommandHistory::replay_untrusted(&tampered, &checkpoint).expect_err("tamper"),
        ArtifactHistoryError::Tampered
    );

    let mut reordered = raw.clone();
    records_mut(&mut reordered).swap(0, 1);
    assert_eq!(
        ArtifactCommandHistory::replay_untrusted(&reordered, &checkpoint).expect_err("reorder"),
        ArtifactHistoryError::Reordered
    );

    let mut truncated = raw.clone();
    records_mut(&mut truncated).pop();
    assert_eq!(
        ArtifactCommandHistory::replay_untrusted(&truncated, &checkpoint).expect_err("truncation"),
        ArtifactHistoryError::Truncated
    );

    let mut duplicate = raw;
    let first = records_mut(&mut duplicate)[0].clone();
    records_mut(&mut duplicate)[1] = first;
    assert_eq!(
        ArtifactCommandHistory::replay_untrusted(&duplicate, &checkpoint).expect_err("duplicate"),
        ArtifactHistoryError::DuplicateCommand
    );
}

#[test]
fn strict_replay_rejects_scope_head_tail_and_denial_tail_substitution() {
    let (_, raw, checkpoint) = two_record_history();

    let mut cross_project = raw.clone();
    let raw_scope = object_field_mut(&mut cross_project, "scope");
    set_string(
        object_field_mut(raw_scope, "project_id"),
        "project-substitute",
    );
    assert_eq!(
        ArtifactCommandHistory::replay_untrusted(&cross_project, &checkpoint)
            .expect_err("cross project"),
        ArtifactHistoryError::ScopeSubstitution
    );

    let mut cross_object = raw.clone();
    let raw_scope = object_field_mut(&mut cross_object, "scope");
    set_string(
        object_field_mut(raw_scope, "content_digest"),
        digest('f').as_str(),
    );
    assert_eq!(
        ArtifactCommandHistory::replay_untrusted(&cross_object, &checkpoint)
            .expect_err("cross object"),
        ArtifactHistoryError::ScopeSubstitution
    );

    let mut high_water = raw.clone();
    let raw_head = object_field_mut(&mut high_water, "head");
    set_string(object_field_mut(raw_head, "high_water"), "1");
    assert_eq!(
        ArtifactCommandHistory::replay_untrusted(&high_water, &checkpoint).expect_err("high water"),
        ArtifactHistoryError::HeadMismatch
    );

    let mut tail = raw.clone();
    let raw_head = object_field_mut(&mut tail, "head");
    set_string(
        object_field_mut(raw_head, "tail_digest"),
        digest('f').as_str(),
    );
    assert_eq!(
        ArtifactCommandHistory::replay_untrusted(&tail, &checkpoint).expect_err("tail"),
        ArtifactHistoryError::HeadMismatch
    );

    let mut denial_tail = raw;
    let raw_head = object_field_mut(&mut denial_tail, "head");
    set_string(object_field_mut(raw_head, "denial_count"), "0");
    *object_field_mut(raw_head, "denial_tail_digest") = CanonicalValue::Null;
    assert_eq!(
        ArtifactCommandHistory::replay_untrusted(&denial_tail, &checkpoint)
            .expect_err("denial tail"),
        ArtifactHistoryError::DenialTailMismatch
    );
}

#[test]
fn coherent_older_prefix_is_rejected_by_the_newer_trusted_checkpoint() {
    let mut history = ArtifactCommandHistory::new();
    history
        .execute(
            request(
                "project-a",
                'a',
                "command-1",
                ArtifactCommandKind::Publish,
                "artifact",
            ),
            || Ok(applied()),
        )
        .expect("first");
    let coherent_old_prefix = history
        .export_untrusted(&scope("project-a", 'a'))
        .expect("old prefix");

    history
        .execute(
            request(
                "project-a",
                'a',
                "command-2",
                ArtifactCommandKind::AddReference,
                "second",
            ),
            || Ok(denied()),
        )
        .expect("second");
    let current_checkpoint = history
        .checkpoint(&scope("project-a", 'a'))
        .expect("checkpoint");

    assert_eq!(
        ArtifactCommandHistory::replay_untrusted(&coherent_old_prefix, &current_checkpoint,)
            .expect_err("old prefix"),
        ArtifactHistoryError::CheckpointMismatch
    );
}

#[test]
fn every_request_record_head_receipt_checkpoint_and_delete_domain_is_distinct() {
    let schema_ids = ArtifactHashDomain::ALL
        .into_iter()
        .map(ArtifactHashDomain::schema_id)
        .collect::<HashSet<_>>();
    assert_eq!(schema_ids.len(), ArtifactHashDomain::ALL.len());

    let same_subject = CanonicalValue::Object(vec![(
        "value".to_owned(),
        CanonicalValue::String("same".to_owned()),
    )]);
    let digests = ArtifactHashDomain::ALL
        .into_iter()
        .map(|domain| domain.digest(&same_subject).expect("domain digest"))
        .collect::<HashSet<_>>();
    assert_eq!(digests.len(), ArtifactHashDomain::ALL.len());
}

#[test]
fn counters_are_bounded_by_postgresql_signed_bigint() {
    let max = ArtifactCounter::new(i64::MAX as u64).expect("signed bigint max");
    ArtifactHistoryCheckpoint::new_trusted(
        scope("project-a", 'a'),
        max,
        Some(digest('b')),
        ArtifactCounter::new(0).expect("zero"),
        None,
    )
    .expect("max signed bigint checkpoint");

    assert!(ArtifactCounter::new(i64::MAX as u64 + 1).is_err());
}

fn two_record_history() -> (
    ArtifactCommandHistory,
    CanonicalValue,
    ArtifactHistoryCheckpoint,
) {
    let mut history = ArtifactCommandHistory::new();
    history
        .execute(
            request(
                "project-a",
                'a',
                "command-1",
                ArtifactCommandKind::Publish,
                "artifact",
            ),
            || Ok(applied()),
        )
        .expect("first");
    history
        .execute(
            request(
                "project-a",
                'a',
                "command-2",
                ArtifactCommandKind::AddReference,
                "second",
            ),
            || Ok(denied()),
        )
        .expect("second");
    let raw = history
        .export_untrusted(&scope("project-a", 'a'))
        .expect("raw");
    let checkpoint = history
        .checkpoint(&scope("project-a", 'a'))
        .expect("checkpoint");
    (history, raw, checkpoint)
}
