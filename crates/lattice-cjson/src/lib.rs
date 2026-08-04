//! Deterministic canonical-byte primitives for LATTICE hash subjects.

use std::error::Error;
use std::fmt;

use sha2::{Digest, Sha256};
use unicode_normalization::UnicodeNormalization;

/// Frozen canonical JSON algorithm identifier.
pub const ALGORITHM_ID: &str = "lattice-cjson-1";
/// Frozen binary hash-frame identifier.
pub const HASH_FRAME_ID: &str = "lattice-hash-1";
/// Initial digest algorithm identifier.
pub const DIGEST_ID: &str = "sha256";

/// A typed value accepted by `lattice-cjson-1`.
///
/// There is intentionally no number variant. Schema owners must validate
/// integer and decimal strings before constructing a canonical value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CanonicalValue {
    /// JSON `null`.
    Null,
    /// JSON Boolean.
    Bool(bool),
    /// Unicode string normalized to NFC during canonicalization.
    String(String),
    /// Ordered JSON array.
    Array(Vec<Self>),
    /// Duplicate-preserving JSON object entries.
    Object(Vec<(String, Self)>),
}

/// A validated schema domain for hash separation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HashDomain {
    schema_id: String,
    schema_version: String,
}

impl HashDomain {
    /// Validates a schema identity and version.
    ///
    /// # Errors
    ///
    /// Returns a typed error for an empty, NUL-bearing, or oversized field.
    pub fn new(
        schema_id: impl Into<String>,
        schema_version: impl Into<String>,
    ) -> Result<Self, CanonicalError> {
        let schema_id = schema_id.into();
        let schema_version = schema_version.into();
        validate_domain_field(&schema_id, "schema_id")?;
        validate_domain_field(&schema_version, "schema_version")?;
        Ok(Self {
            schema_id,
            schema_version,
        })
    }

    /// Returns the schema identifier.
    #[must_use]
    pub fn schema_id(&self) -> &str {
        &self.schema_id
    }

    /// Returns the schema version.
    #[must_use]
    pub fn schema_version(&self) -> &str {
        &self.schema_version
    }
}

/// Canonical UTF-8 JSON bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalBytes(Vec<u8>);

impl CanonicalBytes {
    /// Returns the canonical byte slice.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }

    /// Consumes the wrapper and returns owned bytes.
    #[must_use]
    pub fn into_vec(self) -> Vec<u8> {
        self.0
    }
}

/// One SHA-256 digest.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct Sha256Digest([u8; 32]);

impl Sha256Digest {
    /// Returns the raw 32 digest bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Returns lowercase hexadecimal.
    #[must_use]
    pub fn to_hex(&self) -> String {
        const DIGITS: &[u8; 16] = b"0123456789abcdef";
        let mut output = String::with_capacity(64);
        for byte in self.0 {
            output.push(char::from(DIGITS[usize::from(byte >> 4)]));
            output.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
        }
        output
    }
}

/// Deterministic canonicalization failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CanonicalError {
    /// Two object keys become identical after NFC normalization.
    DuplicateNormalizedKey {
        /// The colliding normalized key.
        key: String,
    },
    /// The schema identifier is empty or contains NUL.
    InvalidSchemaId,
    /// The schema version is empty or contains NUL.
    InvalidSchemaVersion,
    /// A framed field cannot fit its fixed-width length.
    LengthOverflow {
        /// Stable field name.
        field: &'static str,
    },
}

impl CanonicalError {
    /// Returns a stable machine-readable error code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::DuplicateNormalizedKey { .. } => "CJSON_DUPLICATE_NORMALIZED_KEY",
            Self::InvalidSchemaId => "CJSON_INVALID_SCHEMA_ID",
            Self::InvalidSchemaVersion => "CJSON_INVALID_SCHEMA_VERSION",
            Self::LengthOverflow { .. } => "CJSON_LENGTH_OVERFLOW",
        }
    }
}

impl fmt::Display for CanonicalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateNormalizedKey { key } => {
                write!(
                    formatter,
                    "duplicate object key after NFC normalization: {key}"
                )
            }
            Self::InvalidSchemaId => {
                formatter.write_str("schema_id must be non-empty and contain no NUL")
            }
            Self::InvalidSchemaVersion => {
                formatter.write_str("schema_version must be non-empty and contain no NUL")
            }
            Self::LengthOverflow { field } => {
                write!(formatter, "{field} exceeds the lattice-hash-1 length field")
            }
        }
    }
}

impl Error for CanonicalError {}

/// Returns Unicode NFC text.
#[must_use]
pub fn normalize_nfc(value: &str) -> String {
    value.nfc().collect()
}

/// Canonicalizes a typed value to `lattice-cjson-1` UTF-8 bytes.
///
/// # Errors
///
/// Rejects object keys that collide after NFC normalization.
pub fn canonicalize(value: &CanonicalValue) -> Result<CanonicalBytes, CanonicalError> {
    let mut output = String::new();
    write_value(value, &mut output)?;
    Ok(CanonicalBytes(output.into_bytes()))
}

/// Produces the exact `lattice-hash-1` binary frame.
///
/// # Errors
///
/// Returns a canonicalization or fixed-width length error.
pub fn framed_hash_input(
    domain: &HashDomain,
    value: &CanonicalValue,
) -> Result<Vec<u8>, CanonicalError> {
    let canonical = canonicalize(value)?;
    let mut output = Vec::new();
    output.extend_from_slice(HASH_FRAME_ID.as_bytes());
    output.push(0);
    push_u16_field(&mut output, DIGEST_ID.as_bytes(), "digest_id")?;
    push_u16_field(&mut output, ALGORITHM_ID.as_bytes(), "algorithm_id")?;
    push_u16_field(&mut output, domain.schema_id.as_bytes(), "schema_id")?;
    push_u16_field(
        &mut output,
        domain.schema_version.as_bytes(),
        "schema_version",
    )?;
    let payload_length =
        u64::try_from(canonical.as_slice().len()).map_err(|_| CanonicalError::LengthOverflow {
            field: "canonical_bytes",
        })?;
    output.extend_from_slice(&payload_length.to_be_bytes());
    output.extend_from_slice(canonical.as_slice());
    Ok(output)
}

/// Computes SHA-256 over the exact `lattice-hash-1` frame.
///
/// # Errors
///
/// Returns a canonicalization or framing error.
pub fn canonical_sha256(
    domain: &HashDomain,
    value: &CanonicalValue,
) -> Result<Sha256Digest, CanonicalError> {
    let framed = framed_hash_input(domain, value)?;
    let result = Sha256::digest(framed);
    let mut bytes = [0_u8; 32];
    bytes.copy_from_slice(&result);
    Ok(Sha256Digest(bytes))
}

fn validate_domain_field(value: &str, field: &'static str) -> Result<(), CanonicalError> {
    if value.trim().is_empty() || value.contains('\0') {
        return Err(match field {
            "schema_id" => CanonicalError::InvalidSchemaId,
            _ => CanonicalError::InvalidSchemaVersion,
        });
    }
    u16::try_from(value.len())
        .map(|_| ())
        .map_err(|_| CanonicalError::LengthOverflow { field })
}

fn push_u16_field(
    output: &mut Vec<u8>,
    value: &[u8],
    field: &'static str,
) -> Result<(), CanonicalError> {
    let length =
        u16::try_from(value.len()).map_err(|_| CanonicalError::LengthOverflow { field })?;
    output.extend_from_slice(&length.to_be_bytes());
    output.extend_from_slice(value);
    Ok(())
}

fn write_value(value: &CanonicalValue, output: &mut String) -> Result<(), CanonicalError> {
    match value {
        CanonicalValue::Null => output.push_str("null"),
        CanonicalValue::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
        CanonicalValue::String(value) => write_string(&normalize_nfc(value), output),
        CanonicalValue::Array(values) => {
            output.push('[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                write_value(value, output)?;
            }
            output.push(']');
        }
        CanonicalValue::Object(entries) => {
            let mut normalized = entries
                .iter()
                .map(|(key, value)| (normalize_nfc(key), value))
                .collect::<Vec<_>>();
            normalized.sort_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
            for pair in normalized.windows(2) {
                if pair[0].0 == pair[1].0 {
                    return Err(CanonicalError::DuplicateNormalizedKey {
                        key: pair[0].0.clone(),
                    });
                }
            }

            output.push('{');
            for (index, (key, value)) in normalized.iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                write_string(key, output);
                output.push(':');
                write_value(value, output)?;
            }
            output.push('}');
        }
    }
    Ok(())
}

fn write_string(value: &str, output: &mut String) {
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\u{8}' => output.push_str("\\b"),
            '\t' => output.push_str("\\t"),
            '\n' => output.push_str("\\n"),
            '\u{c}' => output.push_str("\\f"),
            '\r' => output.push_str("\\r"),
            control if control <= '\u{1f}' => {
                let code = u32::from(control);
                output.push_str("\\u00");
                output.push(char::from_digit((code >> 4) & 0x0f, 16).expect("hex nibble"));
                output.push(char::from_digit(code & 0x0f, 16).expect("hex nibble"));
            }
            other => output.push(other),
        }
    }
    output.push('"');
}
