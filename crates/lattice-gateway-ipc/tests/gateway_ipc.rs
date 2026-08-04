use lattice_cjson::CanonicalValue;
use lattice_contracts::{ContentDigest, ProjectId, ProjectSnapshotId, SubjectBinding, TaskId};
use lattice_gateway_ipc::{
    CodecErrorKind, MAX_ARRAY_ITEMS, MAX_FRAME_BYTES, MAX_JSON_DEPTH, MAX_JSON_NODES,
    encode_canonical_frame, inspect_canonical_frame, task_spec_document_digest,
    verify_task_spec_document,
};

#[test]
fn frozen_resource_limits_are_exact() {
    assert_eq!(MAX_FRAME_BYTES, 1_048_576);
    assert_eq!(MAX_JSON_DEPTH, 32);
    assert_eq!(MAX_JSON_NODES, 8_192);
    assert_eq!(MAX_ARRAY_ITEMS, 256);
}

#[test]
fn one_byte_over_frame_limit_is_rejected_before_parsing() {
    let exact_bound = vec![b'!'; MAX_FRAME_BYTES];
    let exact_error = inspect_canonical_frame(&exact_bound).unwrap_err();
    assert_eq!(exact_error.kind(), CodecErrorKind::Malformed);

    let oversized = vec![b'!'; MAX_FRAME_BYTES + 1];
    let error = inspect_canonical_frame(&oversized).unwrap_err();
    assert_eq!(error.kind(), CodecErrorKind::FrameTooLarge);
    assert_eq!(error.code(), "GATEWAY_FRAME_TOO_LARGE");
}

#[test]
fn exact_bound_encodes_and_one_byte_over_does_not() {
    let exact = CanonicalValue::String("a".repeat(MAX_FRAME_BYTES - 2));
    assert_eq!(
        encode_canonical_frame(&exact).unwrap().len(),
        MAX_FRAME_BYTES
    );

    let over = CanonicalValue::String("a".repeat(MAX_FRAME_BYTES - 1));
    let error = encode_canonical_frame(&over).unwrap_err();
    assert_eq!(error.kind(), CodecErrorKind::FrameTooLarge);
}

#[test]
fn encode_preflight_accounts_for_escape_expansion_and_structure_limits() {
    let expanded = CanonicalValue::String("\u{1}".repeat((MAX_FRAME_BYTES / 6) + 1));
    assert_eq!(
        encode_canonical_frame(&expanded).unwrap_err().kind(),
        CodecErrorKind::FrameTooLarge
    );

    let oversized_array = CanonicalValue::Array(vec![CanonicalValue::Null; MAX_ARRAY_ITEMS + 1]);
    assert_eq!(
        encode_canonical_frame(&oversized_array).unwrap_err().kind(),
        CodecErrorKind::ArrayLimit
    );

    let mut deep = CanonicalValue::Null;
    for _ in 0..MAX_JSON_DEPTH {
        deep = CanonicalValue::Array(vec![deep]);
    }
    assert_eq!(
        encode_canonical_frame(&deep).unwrap_err().kind(),
        CodecErrorKind::DepthLimit
    );
}

#[test]
fn encode_preflight_rejects_nfc_expansion_before_canonical_allocation() {
    let expanding = "\u{0344}".repeat((MAX_FRAME_BYTES - 2) / 2);
    assert_eq!(expanding.len(), MAX_FRAME_BYTES - 2);
    assert_eq!(
        encode_canonical_frame(&CanonicalValue::String(expanding))
            .unwrap_err()
            .kind(),
        CodecErrorKind::NonCanonical
    );

    let raw_oversized_non_nfc = "\u{0344}".repeat((MAX_FRAME_BYTES / 2) + 1);
    assert_eq!(
        encode_canonical_frame(&CanonicalValue::String(raw_oversized_non_nfc))
            .unwrap_err()
            .kind(),
        CodecErrorKind::FrameTooLarge
    );
}

#[test]
fn invalid_utf8_numbers_and_duplicate_keys_fail_closed() {
    let invalid_utf8 = inspect_canonical_frame(&[0xff]).unwrap_err();
    assert_eq!(invalid_utf8.kind(), CodecErrorKind::InvalidUtf8);

    let number = inspect_canonical_frame(br#"{"value":1}"#).unwrap_err();
    assert_eq!(number.kind(), CodecErrorKind::NumberForbidden);

    let duplicate = inspect_canonical_frame(br#"{"a":"one","a":"two"}"#).unwrap_err();
    assert_eq!(duplicate.kind(), CodecErrorKind::DuplicateKey);

    let nfc_collision =
        inspect_canonical_frame("{\"é\":\"one\",\"e\\u0301\":\"two\"}".as_bytes()).unwrap_err();
    assert_eq!(nfc_collision.kind(), CodecErrorKind::DuplicateKey);
}

#[test]
fn malformed_trailing_and_noncanonical_frames_fail_closed() {
    let malformed = inspect_canonical_frame(br#"{"a":"b"#).unwrap_err();
    assert_eq!(malformed.kind(), CodecErrorKind::Malformed);

    let trailing = inspect_canonical_frame(br#"{"a":"b"}{}"#).unwrap_err();
    assert_eq!(trailing.kind(), CodecErrorKind::TrailingData);

    let key_order = inspect_canonical_frame(br#"{"b":"2","a":"1"}"#).unwrap_err();
    assert_eq!(key_order.kind(), CodecErrorKind::NonCanonical);

    let escape = inspect_canonical_frame(br#"{"a":"\u0062"}"#).unwrap_err();
    assert_eq!(escape.kind(), CodecErrorKind::NonCanonical);
}

#[test]
fn depth_node_and_array_limits_fail_closed() {
    let mut deep = String::new();
    deep.extend(std::iter::repeat_n('[', MAX_JSON_DEPTH + 1));
    deep.push_str("null");
    deep.extend(std::iter::repeat_n(']', MAX_JSON_DEPTH + 1));
    let error = inspect_canonical_frame(deep.as_bytes()).unwrap_err();
    assert_eq!(error.kind(), CodecErrorKind::DepthLimit);

    let array = format!(
        "[{}]",
        std::iter::repeat_n("null", MAX_ARRAY_ITEMS + 1)
            .collect::<Vec<_>>()
            .join(",")
    );
    let error = inspect_canonical_frame(array.as_bytes()).unwrap_err();
    assert_eq!(error.kind(), CodecErrorKind::ArrayLimit);

    let nodes = format!(
        "{{{}}}",
        (0..MAX_JSON_NODES)
            .map(|index| format!("\"k{index:05}\":null"))
            .collect::<Vec<_>>()
            .join(",")
    );
    let error = inspect_canonical_frame(nodes.as_bytes()).unwrap_err();
    assert_eq!(error.kind(), CodecErrorKind::NodeLimit);
}

#[test]
fn codec_error_debug_and_display_are_bounded_and_redacted() {
    let secret = "TASK_SPEC_SECRET_MUST_NOT_LEAK";
    let input = format!("{{\"document\":\"{secret}\",\"value\":1}}");
    let error = inspect_canonical_frame(input.as_bytes()).unwrap_err();
    let debug = format!("{error:?}");
    let display = error.to_string();
    assert!(!debug.contains(secret));
    assert!(!display.contains(secret));
    assert!(debug.len() <= 160);
    assert!(display.len() <= 160);
}

#[test]
fn task_spec_digest_and_exact_binding_are_checked_without_domain_validation() {
    let document = br#"{"project_id":"project-a","project_snapshot_id":"snapshot-a","revision":"1","schema_version":"2.1","task_id":"task-a"}"#;
    let digest = task_spec_document_digest(document).unwrap();
    let binding = SubjectBinding::new(
        ProjectId::new("project-a").unwrap(),
        ProjectSnapshotId::new("snapshot-a").unwrap(),
        TaskId::new("task-a").unwrap(),
        "1",
        digest.clone(),
    )
    .unwrap();
    verify_task_spec_document(document, &digest, &binding).unwrap();

    let wrong_claim = ContentDigest::from_sha256("a".repeat(64)).unwrap();
    let error = verify_task_spec_document(document, &wrong_claim, &binding).unwrap_err();
    assert_eq!(error.kind(), CodecErrorKind::DigestMismatch);

    let wrong_binding = SubjectBinding::new(
        ProjectId::new("project-b").unwrap(),
        ProjectSnapshotId::new("snapshot-a").unwrap(),
        TaskId::new("task-a").unwrap(),
        "1",
        digest.clone(),
    )
    .unwrap();
    let error = verify_task_spec_document(document, &digest, &wrong_binding).unwrap_err();
    assert_eq!(error.kind(), CodecErrorKind::BindingMismatch);

    let raw_debug = format!(
        "{:?}",
        verify_task_spec_document(
            br#"{"goal":"TASK_SPEC_SECRET","schema_version":"2.1"}"#,
            &digest,
            &binding,
        )
        .unwrap_err()
    );
    assert!(!raw_debug.contains("TASK_SPEC_SECRET"));
}
