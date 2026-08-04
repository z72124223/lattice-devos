use lattice_cjson::{
    CanonicalError, CanonicalValue, HashDomain, canonical_sha256, canonicalize, framed_hash_input,
};

fn text(value: &str) -> CanonicalValue {
    CanonicalValue::String(value.to_owned())
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(char::from(DIGITS[usize::from(byte >> 4)]));
        result.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    result
}

#[test]
fn canonicalizes_nfc_and_sorts_keys_by_normalized_utf8_bytes() {
    let value = CanonicalValue::Object(vec![
        ("\u{10000}".to_owned(), text("astral")),
        ("\u{e000}".to_owned(), text("bmp")),
        ("e\u{301}".to_owned(), text("Cafe\u{301}")),
    ]);

    let canonical = canonicalize(&value).expect("valid canonical value");
    let expected = "{\"é\":\"Café\",\"\u{e000}\":\"bmp\",\"\u{10000}\":\"astral\"}";

    assert_eq!(canonical.as_slice(), expected.as_bytes());
}

#[test]
fn duplicate_keys_after_nfc_normalization_are_rejected() {
    let value = CanonicalValue::Object(vec![
        ("é".to_owned(), CanonicalValue::Null),
        ("e\u{301}".to_owned(), CanonicalValue::Bool(true)),
    ]);

    let error = canonicalize(&value).expect_err("NFC collision must fail");

    assert_eq!(error.code(), "CJSON_DUPLICATE_NORMALIZED_KEY");
    assert!(matches!(
        error,
        CanonicalError::DuplicateNormalizedKey { ref key } if key == "é"
    ));
}

#[test]
fn emits_minimal_escaping_and_literal_unicode() {
    let value = text("\"\u{5c}\u{8}\t\n\u{c}\r\u{0}\u{1}/\u{2028}\u{2029}");

    let canonical = canonicalize(&value).expect("valid canonical string");
    let expected = "\"\\\"\\\\\\b\\t\\n\\f\\r\\u0000\\u0001/\u{2028}\u{2029}\"";

    assert_eq!(canonical.as_slice(), expected.as_bytes());
}

#[test]
fn preserves_array_order_and_distinguishes_null_from_missing() {
    let array = CanonicalValue::Array(vec![
        CanonicalValue::Bool(true),
        CanonicalValue::Bool(false),
        CanonicalValue::Null,
        text("18446744073709551615"),
        text("12.34"),
        text("2026-07-29T00:00:00.12Z"),
    ]);
    let missing = CanonicalValue::Object(vec![]);
    let explicit_null = CanonicalValue::Object(vec![("value".to_owned(), CanonicalValue::Null)]);

    assert_eq!(
        canonicalize(&array).expect("array").as_slice(),
        b"[true,false,null,\"18446744073709551615\",\"12.34\",\"2026-07-29T00:00:00.12Z\"]"
    );
    assert_eq!(canonicalize(&missing).expect("missing").as_slice(), b"{}");
    assert_eq!(
        canonicalize(&explicit_null)
            .expect("explicit null")
            .as_slice(),
        b"{\"value\":null}"
    );
}

#[test]
fn object_insertion_order_does_not_change_bytes() {
    let first = CanonicalValue::Object(vec![
        ("b".to_owned(), text("2")),
        ("a".to_owned(), text("1")),
    ]);
    let second = CanonicalValue::Object(vec![
        ("a".to_owned(), text("1")),
        ("b".to_owned(), text("2")),
    ]);

    assert_eq!(
        canonicalize(&first).expect("first"),
        canonicalize(&second).expect("second")
    );
}

#[test]
fn freezes_lattice_hash_frame_and_digest() {
    let domain = HashDomain::new("lattice.test", "1").expect("valid domain");
    let value = CanonicalValue::Object(vec![
        ("n".to_owned(), CanonicalValue::Null),
        ("a".to_owned(), text("e\u{301}")),
    ]);

    let frame = framed_hash_input(&domain, &value).expect("valid frame");
    let digest = canonical_sha256(&domain, &value).expect("valid digest");

    assert_eq!(
        hex(&frame),
        "6c6174746963652d686173682d31000006736861323536000f6c6174746963652d636a736f6e2d31000c6c6174746963652e7465737400013100000000000000137b2261223a22c3a9222c226e223a6e756c6c7d"
    );
    assert_eq!(
        digest.to_hex(),
        "d136cf215029a2ee3ede2e6e0c6c15da8a14293c5e0841fddef3bbb6c92fc623"
    );
}

#[test]
fn schema_identity_and_version_are_domain_separated() {
    let value = text("same");
    let base = canonical_sha256(
        &HashDomain::new("lattice.task-spec", "2.0").expect("domain"),
        &value,
    )
    .expect("hash");
    let other_schema = canonical_sha256(
        &HashDomain::new("lattice.event", "2.0").expect("domain"),
        &value,
    )
    .expect("hash");
    let other_version = canonical_sha256(
        &HashDomain::new("lattice.task-spec", "2.1").expect("domain"),
        &value,
    )
    .expect("hash");

    assert_ne!(base, other_schema);
    assert_ne!(base, other_version);
}

#[test]
fn invalid_or_oversized_hash_domains_fail_closed() {
    assert_eq!(
        HashDomain::new("", "1")
            .expect_err("empty schema id")
            .code(),
        "CJSON_INVALID_SCHEMA_ID"
    );
    assert_eq!(
        HashDomain::new("lattice.test", "\0")
            .expect_err("NUL version")
            .code(),
        "CJSON_INVALID_SCHEMA_VERSION"
    );
    assert_eq!(
        HashDomain::new("x".repeat(usize::from(u16::MAX) + 1), "1")
            .expect_err("oversized schema id")
            .code(),
        "CJSON_LENGTH_OVERFLOW"
    );
}
