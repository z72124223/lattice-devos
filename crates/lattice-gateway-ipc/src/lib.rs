//! Pure, bounded codec and in-memory loopback for the LATTICE gateway protocol.

use std::collections::BTreeSet;
use std::error::Error;
use std::fmt;

use lattice_cjson::{CanonicalValue, HashDomain, canonical_sha256, canonicalize, normalize_nfc};
use lattice_contracts::{
    ContentDigest, GATEWAY_TASK_SPEC_MAX_BYTES, GATEWAY_TASK_SPEC_SCHEMA_ID,
    GATEWAY_TASK_SPEC_SCHEMA_VERSION, SubjectBinding,
};
use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use unicode_normalization::is_nfc;

mod fake;
mod typed;

pub use fake::{
    FakeFault, FakeGatewayClient, FakeGatewayServer, LoopbackError, LoopbackErrorKind,
    MAX_REPLAY_ENTRIES,
};
pub use typed::{
    build_reply, build_request, decode_reply, decode_request, encode_reply, encode_request,
};

/// Maximum accepted raw request or reply frame size.
pub const MAX_FRAME_BYTES: usize = 1_048_576;
/// Maximum JSON nesting depth accepted by the bounded parser.
pub const MAX_JSON_DEPTH: usize = 32;
/// Maximum aggregate JSON nodes accepted by the bounded parser.
pub const MAX_JSON_NODES: usize = 8_192;
/// Maximum members accepted in any JSON array.
pub const MAX_ARRAY_ITEMS: usize = 256;

const NUMBER_MARKER: &str = "LATTICE_NUMBER_FORBIDDEN";
const DUPLICATE_MARKER: &str = "LATTICE_DUPLICATE_KEY";
const DEPTH_MARKER: &str = "LATTICE_DEPTH_LIMIT";
const NODE_MARKER: &str = "LATTICE_NODE_LIMIT";
const ARRAY_MARKER: &str = "LATTICE_ARRAY_LIMIT";

/// Stable class of canonical-frame rejection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodecErrorKind {
    /// Raw frame length exceeded [`MAX_FRAME_BYTES`].
    FrameTooLarge,
    /// The raw frame was not UTF-8.
    InvalidUtf8,
    /// JSON numbers are forbidden by `lattice-cjson-1`.
    NumberForbidden,
    /// An object repeated a key, including after NFC normalization.
    DuplicateKey,
    /// The frame was not one complete JSON value.
    Malformed,
    /// Non-whitespace data followed the first complete JSON value.
    TrailingData,
    /// The parsed value or encoder input did not use its canonical NFC form.
    NonCanonical,
    /// JSON nesting exceeded [`MAX_JSON_DEPTH`].
    DepthLimit,
    /// Aggregate JSON values exceeded [`MAX_JSON_NODES`].
    NodeLimit,
    /// One array exceeded [`MAX_ARRAY_ITEMS`].
    ArrayLimit,
    /// A Task Spec document exceeded its independent carrier cap.
    DocumentTooLarge,
    /// A claimed domain-separated digest did not match the canonical content.
    DigestMismatch,
    /// Task Spec identity fields did not match the exact request binding.
    BindingMismatch,
    /// The Task Spec document did not name schema version 2.1.
    UnsupportedTaskSpecVersion,
    /// An object carried a field outside its frozen schema.
    UnknownField,
    /// An object omitted one required frozen-schema field.
    MissingField,
    /// The protocol identifier was not `lattice-gateway-ipc`.
    UnsupportedProtocol,
    /// The protocol version was not supported.
    UnsupportedVersion,
    /// The action was not one of the six closed gateway actions.
    UnknownAction,
    /// A field violated its bounded typed representation.
    InvalidField,
    /// An action and its payload shape disagreed.
    ShapeMismatch,
    /// A reply was not bound to the exact request.
    ReplyMismatch,
}

/// A bounded and payload-free codec rejection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CodecError {
    kind: CodecErrorKind,
}

impl CodecError {
    pub(crate) const fn new(kind: CodecErrorKind) -> Self {
        Self { kind }
    }

    /// Returns the stable rejection class.
    #[must_use]
    pub const fn kind(self) -> CodecErrorKind {
        self.kind
    }

    /// Returns a stable machine-facing code without echoing input bytes.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self.kind {
            CodecErrorKind::FrameTooLarge => "GATEWAY_FRAME_TOO_LARGE",
            CodecErrorKind::InvalidUtf8 => "GATEWAY_INVALID_UTF8",
            CodecErrorKind::NumberForbidden => "GATEWAY_NUMBER_FORBIDDEN",
            CodecErrorKind::DuplicateKey => "GATEWAY_DUPLICATE_KEY",
            CodecErrorKind::Malformed => "GATEWAY_MALFORMED_JSON",
            CodecErrorKind::TrailingData => "GATEWAY_TRAILING_DATA",
            CodecErrorKind::NonCanonical => "GATEWAY_NONCANONICAL_JSON",
            CodecErrorKind::DepthLimit => "GATEWAY_DEPTH_LIMIT",
            CodecErrorKind::NodeLimit => "GATEWAY_NODE_LIMIT",
            CodecErrorKind::ArrayLimit => "GATEWAY_ARRAY_LIMIT",
            CodecErrorKind::DocumentTooLarge => "GATEWAY_TASK_SPEC_TOO_LARGE",
            CodecErrorKind::DigestMismatch => "GATEWAY_DIGEST_MISMATCH",
            CodecErrorKind::BindingMismatch => "GATEWAY_BINDING_MISMATCH",
            CodecErrorKind::UnsupportedTaskSpecVersion => "GATEWAY_TASK_SPEC_VERSION_UNSUPPORTED",
            CodecErrorKind::UnknownField => "GATEWAY_UNKNOWN_FIELD",
            CodecErrorKind::MissingField => "GATEWAY_MISSING_FIELD",
            CodecErrorKind::UnsupportedProtocol => "GATEWAY_PROTOCOL_UNSUPPORTED",
            CodecErrorKind::UnsupportedVersion => "GATEWAY_VERSION_UNSUPPORTED",
            CodecErrorKind::UnknownAction => "GATEWAY_ACTION_UNKNOWN",
            CodecErrorKind::InvalidField => "GATEWAY_INVALID_FIELD",
            CodecErrorKind::ShapeMismatch => "GATEWAY_SHAPE_MISMATCH",
            CodecErrorKind::ReplyMismatch => "GATEWAY_REPLY_MISMATCH",
        }
    }
}

impl fmt::Display for CodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl Error for CodecError {}

/// Parses one bounded canonical `lattice-cjson-1` value.
///
/// The raw-size check is deliberately the first operation. The returned value
/// is syntax only; this function does not interpret Task Spec semantics.
///
/// # Errors
///
/// Rejects oversized, non-UTF-8, numeric, duplicate-key, malformed,
/// non-canonical, over-deep, over-node, and over-array frames.
pub fn inspect_canonical_frame(input: &[u8]) -> Result<(), CodecError> {
    parse_canonical_frame(input).map(drop)
}

pub(crate) fn parse_canonical_frame(input: &[u8]) -> Result<CanonicalValue, CodecError> {
    if input.len() > MAX_FRAME_BYTES {
        return Err(CodecError::new(CodecErrorKind::FrameTooLarge));
    }
    let text =
        std::str::from_utf8(input).map_err(|_| CodecError::new(CodecErrorKind::InvalidUtf8))?;
    let mut stats = ParseStats { nodes: 0 };
    let mut deserializer = serde_json::Deserializer::from_str(text);
    let value = ValueSeed {
        depth: 1,
        stats: &mut stats,
    }
    .deserialize(&mut deserializer)
    .map_err(|error| classify_parse_error(&error))?;
    deserializer
        .end()
        .map_err(|_| CodecError::new(CodecErrorKind::TrailingData))?;
    let canonical =
        canonicalize(&value).map_err(|_| CodecError::new(CodecErrorKind::DuplicateKey))?;
    if canonical.as_slice() != input {
        return Err(CodecError::new(CodecErrorKind::NonCanonical));
    }
    Ok(value)
}

/// Encodes one typed canonical value and applies the same complete-frame cap.
///
/// # Errors
///
/// Rejects non-NFC input, a duplicate normalized key, or an encoded frame
/// above one MiB. NFC is checked without allocating a normalized replacement.
pub fn encode_canonical_frame(value: &CanonicalValue) -> Result<Vec<u8>, CodecError> {
    preflight_encode_value(value)?;
    let bytes = canonicalize(value)
        .map_err(|_| CodecError::new(CodecErrorKind::DuplicateKey))?
        .into_vec();
    if bytes.len() > MAX_FRAME_BYTES {
        return Err(CodecError::new(CodecErrorKind::FrameTooLarge));
    }
    Ok(bytes)
}

fn preflight_encode_value(root: &CanonicalValue) -> Result<(), CodecError> {
    let mut stack = vec![(root, 1_usize)];
    let mut nodes = 0_usize;
    let mut encoded_bytes = 0_usize;
    while let Some((value, depth)) = stack.pop() {
        if depth > MAX_JSON_DEPTH {
            return Err(CodecError::new(CodecErrorKind::DepthLimit));
        }
        nodes = nodes.saturating_add(1);
        if nodes > MAX_JSON_NODES {
            return Err(CodecError::new(CodecErrorKind::NodeLimit));
        }
        match value {
            CanonicalValue::Null => add_encoded_bytes(&mut encoded_bytes, 4)?,
            CanonicalValue::Bool(value) => {
                add_encoded_bytes(&mut encoded_bytes, if *value { 4 } else { 5 })?;
            }
            CanonicalValue::String(value) => {
                if value.len().saturating_add(2) > MAX_FRAME_BYTES {
                    return Err(CodecError::new(CodecErrorKind::FrameTooLarge));
                }
                if !is_nfc(value) {
                    return Err(CodecError::new(CodecErrorKind::NonCanonical));
                }
                add_encoded_bytes(&mut encoded_bytes, encoded_string_bytes(value)?)?;
            }
            CanonicalValue::Array(values) => {
                if values.len() > MAX_ARRAY_ITEMS {
                    return Err(CodecError::new(CodecErrorKind::ArrayLimit));
                }
                add_encoded_bytes(
                    &mut encoded_bytes,
                    2_usize.saturating_add(values.len().saturating_sub(1)),
                )?;
                stack.extend(
                    values
                        .iter()
                        .rev()
                        .map(|value| (value, depth.saturating_add(1))),
                );
            }
            CanonicalValue::Object(entries) => {
                nodes = nodes.saturating_add(entries.len());
                if nodes > MAX_JSON_NODES {
                    return Err(CodecError::new(CodecErrorKind::NodeLimit));
                }
                add_encoded_bytes(
                    &mut encoded_bytes,
                    2_usize.saturating_add(entries.len().saturating_sub(1)),
                )?;
                for (key, value) in entries.iter().rev() {
                    if key.len().saturating_add(2) > MAX_FRAME_BYTES {
                        return Err(CodecError::new(CodecErrorKind::FrameTooLarge));
                    }
                    if !is_nfc(key) {
                        return Err(CodecError::new(CodecErrorKind::NonCanonical));
                    }
                    add_encoded_bytes(
                        &mut encoded_bytes,
                        encoded_string_bytes(key)?.saturating_add(1),
                    )?;
                    stack.push((value, depth.saturating_add(1)));
                }
            }
        }
    }
    Ok(())
}

fn encoded_string_bytes(value: &str) -> Result<usize, CodecError> {
    let mut length = 2_usize;
    for character in value.chars() {
        let encoded = match character {
            '"' | '\\' | '\u{8}' | '\t' | '\n' | '\u{c}' | '\r' => 2,
            control if control <= '\u{1f}' => 6,
            other => other.len_utf8(),
        };
        length = length
            .checked_add(encoded)
            .ok_or_else(|| CodecError::new(CodecErrorKind::FrameTooLarge))?;
        if length > MAX_FRAME_BYTES {
            return Err(CodecError::new(CodecErrorKind::FrameTooLarge));
        }
    }
    Ok(length)
}

fn add_encoded_bytes(total: &mut usize, amount: usize) -> Result<(), CodecError> {
    *total = total
        .checked_add(amount)
        .ok_or_else(|| CodecError::new(CodecErrorKind::FrameTooLarge))?;
    if *total > MAX_FRAME_BYTES {
        return Err(CodecError::new(CodecErrorKind::FrameTooLarge));
    }
    Ok(())
}

/// Recomputes the Task Spec 2.1 domain-separated digest without validating its
/// domain semantics.
///
/// # Errors
///
/// Rejects an oversized or non-canonical document and any canonical hashing
/// failure without returning source bytes.
pub fn task_spec_document_digest(document: &[u8]) -> Result<ContentDigest, CodecError> {
    if document.len() > GATEWAY_TASK_SPEC_MAX_BYTES {
        return Err(CodecError::new(CodecErrorKind::DocumentTooLarge));
    }
    let value = parse_canonical_frame(document)?;
    let domain = HashDomain::new(
        GATEWAY_TASK_SPEC_SCHEMA_ID,
        GATEWAY_TASK_SPEC_SCHEMA_VERSION,
    )
    .map_err(|_| CodecError::new(CodecErrorKind::Malformed))?;
    let digest = canonical_sha256(&domain, &value)
        .map_err(|_| CodecError::new(CodecErrorKind::Malformed))?;
    ContentDigest::from_sha256(digest.to_hex())
        .map_err(|_| CodecError::new(CodecErrorKind::Malformed))
}

/// Mechanically checks the Task Spec digest and the five identity bindings
/// shared with the gateway envelope.
///
/// This deliberately does not validate the remaining Task Spec fields; Task
/// Domain remains their only semantic owner.
///
/// # Errors
///
/// Rejects a malformed document, digest mismatch, unsupported schema version,
/// or exact project/snapshot/task/revision binding mismatch.
pub fn verify_task_spec_document(
    document: &[u8],
    claimed_digest: &ContentDigest,
    binding: &SubjectBinding,
) -> Result<(), CodecError> {
    let value = inspect_task_spec_value(document)?;
    let domain = HashDomain::new(
        GATEWAY_TASK_SPEC_SCHEMA_ID,
        GATEWAY_TASK_SPEC_SCHEMA_VERSION,
    )
    .map_err(|_| CodecError::new(CodecErrorKind::Malformed))?;
    let actual = canonical_sha256(&domain, &value)
        .map_err(|_| CodecError::new(CodecErrorKind::Malformed))?
        .to_hex();
    if actual != claimed_digest.as_str() || binding.task_spec_digest() != claimed_digest {
        return Err(CodecError::new(CodecErrorKind::DigestMismatch));
    }

    let CanonicalValue::Object(fields) = &value else {
        return Err(CodecError::new(CodecErrorKind::BindingMismatch));
    };
    if object_string(fields, "schema_version") != Some(GATEWAY_TASK_SPEC_SCHEMA_VERSION) {
        return Err(CodecError::new(CodecErrorKind::UnsupportedTaskSpecVersion));
    }
    let binding_matches = object_string(fields, "project_id")
        == Some(binding.project_id().as_str())
        && object_string(fields, "project_snapshot_id")
            == Some(binding.project_snapshot_id().as_str())
        && object_string(fields, "task_id") == Some(binding.task_id().as_str())
        && object_string(fields, "revision") == Some(binding.task_revision());
    if !binding_matches {
        return Err(CodecError::new(CodecErrorKind::BindingMismatch));
    }
    Ok(())
}

fn inspect_task_spec_value(document: &[u8]) -> Result<CanonicalValue, CodecError> {
    if document.len() > GATEWAY_TASK_SPEC_MAX_BYTES {
        return Err(CodecError::new(CodecErrorKind::DocumentTooLarge));
    }
    parse_canonical_frame(document)
}

fn object_string<'a>(fields: &'a [(String, CanonicalValue)], key: &str) -> Option<&'a str> {
    fields.iter().find_map(|(candidate, value)| {
        (candidate == key)
            .then_some(value)
            .and_then(|value| match value {
                CanonicalValue::String(value) => Some(value.as_str()),
                _ => None,
            })
    })
}

struct ParseStats {
    nodes: usize,
}

struct ValueSeed<'a> {
    depth: usize,
    stats: &'a mut ParseStats,
}

impl<'de> DeserializeSeed<'de> for ValueSeed<'_> {
    type Value = CanonicalValue;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        if self.depth > MAX_JSON_DEPTH {
            return Err(de::Error::custom(DEPTH_MARKER));
        }
        self.stats.nodes = self.stats.nodes.saturating_add(1);
        if self.stats.nodes > MAX_JSON_NODES {
            return Err(de::Error::custom(NODE_MARKER));
        }
        deserializer.deserialize_any(ValueVisitor {
            depth: self.depth,
            stats: self.stats,
        })
    }
}

struct ValueVisitor<'a> {
    depth: usize,
    stats: &'a mut ParseStats,
}

impl<'de> Visitor<'de> for ValueVisitor<'_> {
    type Value = CanonicalValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a bounded lattice-cjson-1 value")
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(CanonicalValue::Null)
    }

    fn visit_none<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(CanonicalValue::Null)
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(CanonicalValue::Bool(value))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(CanonicalValue::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(CanonicalValue::String(value))
    }

    fn visit_i64<E>(self, _value: i64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Err(E::custom(NUMBER_MARKER))
    }

    fn visit_u64<E>(self, _value: u64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Err(E::custom(NUMBER_MARKER))
    }

    fn visit_f64<E>(self, _value: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Err(E::custom(NUMBER_MARKER))
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element_seed(ValueSeed {
            depth: self.depth.saturating_add(1),
            stats: self.stats,
        })? {
            if values.len() == MAX_ARRAY_ITEMS {
                return Err(de::Error::custom(ARRAY_MARKER));
            }
            values.push(value);
        }
        Ok(CanonicalValue::Array(values))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut entries = Vec::new();
        let mut normalized_keys = BTreeSet::<String>::new();
        while let Some(key) = map.next_key::<String>()? {
            let normalized = normalize_nfc(&key);
            if !normalized_keys.insert(normalized) {
                return Err(de::Error::custom(DUPLICATE_MARKER));
            }
            self.stats.nodes = self.stats.nodes.saturating_add(1);
            if self.stats.nodes > MAX_JSON_NODES {
                return Err(de::Error::custom(NODE_MARKER));
            }
            let value = map.next_value_seed(ValueSeed {
                depth: self.depth.saturating_add(1),
                stats: self.stats,
            })?;
            entries.push((key, value));
        }
        Ok(CanonicalValue::Object(entries))
    }
}

fn classify_parse_error(error: &serde_json::Error) -> CodecError {
    let message = error.to_string();
    let kind = if message.contains(NUMBER_MARKER) {
        CodecErrorKind::NumberForbidden
    } else if message.contains(DUPLICATE_MARKER) {
        CodecErrorKind::DuplicateKey
    } else if message.contains(DEPTH_MARKER) {
        CodecErrorKind::DepthLimit
    } else if message.contains(NODE_MARKER) {
        CodecErrorKind::NodeLimit
    } else if message.contains(ARRAY_MARKER) {
        CodecErrorKind::ArrayLimit
    } else {
        CodecErrorKind::Malformed
    };
    CodecError::new(kind)
}
