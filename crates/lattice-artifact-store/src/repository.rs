//! Component-free durable metadata repository boundary.

use std::error::Error;
use std::fmt;

use lattice_cjson::{CanonicalValue, canonicalize, normalize_nfc};
use lattice_contracts::ContentDigest;

use crate::{
    ArtifactStoreCheckpoint, ArtifactStoreIdentity, ArtifactStoreReplayError, FakeArtifactStore,
};

/// Absolute canonical-byte bound for one durable metadata snapshot.
pub const MAX_REPOSITORY_SNAPSHOT_BYTES: usize = 64 * 1_048_576;
/// Absolute canonical-byte bound for one independently retained checkpoint.
pub const MAX_REPOSITORY_CHECKPOINT_BYTES: usize = 16_384;
const MAX_CANONICAL_NESTING_DEPTH: usize = 128;

/// Closed durable repository failure classification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArtifactRepositoryErrorKind {
    /// Pure snapshot/checkpoint construction failed.
    Domain,
    /// The persistence component was unavailable.
    Unavailable,
    /// Bounded serialization retries were exhausted.
    SerializationExhausted,
    /// The server may have committed but no trustworthy response was received.
    CommitOutcomeUnknown,
    /// Stored bytes, checkpoint fields, or physical history were corrupt.
    Corrupt,
    /// The expected checkpoint was no longer current.
    StaleWrite,
    /// Physical target identity or runtime authority did not match.
    AuthorityMismatch,
}

/// Redacted repository error carrying only a stable closed classification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArtifactRepositoryError {
    kind: ArtifactRepositoryErrorKind,
}

impl ArtifactRepositoryError {
    /// Constructs one stable repository failure.
    #[must_use]
    pub const fn new(kind: ArtifactRepositoryErrorKind) -> Self {
        Self { kind }
    }

    /// Maps a pure replay/construction error without exposing raw bytes.
    #[must_use]
    pub const fn from_replay(_error: &ArtifactStoreReplayError) -> Self {
        Self::new(ArtifactRepositoryErrorKind::Domain)
    }

    /// Returns the closed failure kind.
    #[must_use]
    pub const fn kind(self) -> ArtifactRepositoryErrorKind {
        self.kind
    }

    /// Returns the stable non-secret diagnostic code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self.kind {
            ArtifactRepositoryErrorKind::Domain => "ARTIFACT_REPOSITORY_DOMAIN",
            ArtifactRepositoryErrorKind::Unavailable => "ARTIFACT_REPOSITORY_UNAVAILABLE",
            ArtifactRepositoryErrorKind::SerializationExhausted => {
                "ARTIFACT_REPOSITORY_SERIALIZATION_EXHAUSTED"
            }
            ArtifactRepositoryErrorKind::CommitOutcomeUnknown => {
                "ARTIFACT_REPOSITORY_COMMIT_OUTCOME_UNKNOWN"
            }
            ArtifactRepositoryErrorKind::Corrupt => "ARTIFACT_REPOSITORY_CORRUPT",
            ArtifactRepositoryErrorKind::StaleWrite => "ARTIFACT_REPOSITORY_STALE_WRITE",
            ArtifactRepositoryErrorKind::AuthorityMismatch => {
                "ARTIFACT_REPOSITORY_AUTHORITY_MISMATCH"
            }
        }
    }
}

impl fmt::Display for ArtifactRepositoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl Error for ArtifactRepositoryError {}

/// Exact canonical metadata plus an independently serialized checkpoint.
/// Artifact payload bytes are physically absent.
#[derive(Clone, Eq, PartialEq)]
pub struct ArtifactRepositorySnapshot {
    store_id: ArtifactStoreIdentity,
    snapshot_bytes: Vec<u8>,
    checkpoint_bytes: Vec<u8>,
    checkpoint_digest: ContentDigest,
}

impl fmt::Debug for ArtifactRepositorySnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ArtifactRepositorySnapshot")
            .field("store_id", &self.store_id)
            .field("snapshot_byte_length", &self.snapshot_bytes.len())
            .field("checkpoint_byte_length", &self.checkpoint_bytes.len())
            .field("checkpoint_digest", &self.checkpoint_digest)
            .field("payload_bytes", &"[ABSENT]")
            .finish()
    }
}

impl ArtifactRepositorySnapshot {
    /// Captures one complete bounded metadata snapshot and independent
    /// checkpoint from the pure owner.
    ///
    /// # Errors
    ///
    /// Rejects inconsistent owner state, canonicalization failure, or a
    /// repository byte bound.
    pub fn capture(store: &FakeArtifactStore) -> Result<Self, ArtifactStoreReplayError> {
        let snapshot = store.export_untrusted()?;
        let snapshot_bytes = repository_canonical_bytes(&snapshot)?;
        if snapshot_bytes.is_empty() || snapshot_bytes.len() > MAX_REPOSITORY_SNAPSHOT_BYTES {
            return Err(ArtifactStoreReplayError::ReplayLimit {
                field: "repository_snapshot_bytes",
            });
        }
        let checkpoint = store.checkpoint()?;
        let checkpoint_bytes = checkpoint.repository_canonical_bytes()?;
        Ok(Self {
            store_id: store.store_id().clone(),
            snapshot_bytes,
            checkpoint_bytes,
            checkpoint_digest: checkpoint.checkpoint_digest().clone(),
        })
    }

    /// Strictly reconstructs and verifies one repository snapshot.
    ///
    /// # Errors
    ///
    /// Rejects empty, oversized, non-canonical, malformed, substituted,
    /// corrupt, or rollback-inconsistent bytes.
    pub fn from_canonical_bytes(
        snapshot_bytes: &[u8],
        checkpoint_bytes: &[u8],
    ) -> Result<Self, ArtifactStoreReplayError> {
        let raw = parse_canonical_bytes(snapshot_bytes, MAX_REPOSITORY_SNAPSHOT_BYTES)?;
        let checkpoint =
            ArtifactStoreCheckpoint::from_repository_canonical_bytes(checkpoint_bytes)?;
        let restored = FakeArtifactStore::replay_untrusted(&raw, &checkpoint)?;
        let captured = Self {
            store_id: restored.store_id().clone(),
            snapshot_bytes: snapshot_bytes.to_vec(),
            checkpoint_bytes: checkpoint_bytes.to_vec(),
            checkpoint_digest: checkpoint.checkpoint_digest().clone(),
        };
        if captured.snapshot_bytes != repository_canonical_bytes(&raw)? {
            return Err(ArtifactStoreReplayError::SnapshotMismatch);
        }
        Ok(captured)
    }

    /// Replays the stored metadata into a fresh byte-empty semantic owner.
    ///
    /// # Errors
    ///
    /// Rejects any byte or checkpoint drift since construction.
    pub fn replay(&self) -> Result<FakeArtifactStore, ArtifactStoreReplayError> {
        let raw = parse_canonical_bytes(&self.snapshot_bytes, MAX_REPOSITORY_SNAPSHOT_BYTES)?;
        let checkpoint =
            ArtifactStoreCheckpoint::from_repository_canonical_bytes(&self.checkpoint_bytes)?;
        if checkpoint.store_id() != &self.store_id
            || checkpoint.checkpoint_digest() != &self.checkpoint_digest
        {
            return Err(ArtifactStoreReplayError::SnapshotMismatch);
        }
        FakeArtifactStore::replay_untrusted(&raw, &checkpoint)
    }

    /// Verifies that this snapshot is the exact vacant state for its identity
    /// and immutable limits.
    ///
    /// # Errors
    ///
    /// Rejects any non-empty or internally inconsistent initial snapshot.
    pub fn verify_initial(&self) -> Result<(), ArtifactStoreReplayError> {
        self.replay()?
            .validate_repository_initial()
            .map_err(|_| ArtifactStoreReplayError::InvalidMetadata)
    }

    /// Verifies that `next` is exactly one owner-produced semantic transition
    /// after this snapshot while retaining all immutable prior receipts.
    ///
    /// # Errors
    ///
    /// Rejects identity/limit drift, history replacement, multi-command jumps,
    /// or a next-state commitment that does not match the added command.
    pub fn verify_successor(&self, next: &Self) -> Result<(), ArtifactStoreReplayError> {
        let current = self.replay()?;
        let next = next.replay()?;
        current
            .validate_repository_successor(&next)
            .map_err(|_| ArtifactStoreReplayError::InvalidMetadata)
    }

    /// Exact store namespace bound by both documents.
    #[must_use]
    pub const fn store_id(&self) -> &ArtifactStoreIdentity {
        &self.store_id
    }

    /// Exact untrusted canonical metadata bytes.
    #[must_use]
    pub fn snapshot_bytes(&self) -> &[u8] {
        &self.snapshot_bytes
    }

    /// Exact independently retained canonical checkpoint bytes.
    #[must_use]
    pub fn checkpoint_bytes(&self) -> &[u8] {
        &self.checkpoint_bytes
    }

    /// Independent checkpoint digest used by compare-and-swap.
    #[must_use]
    pub const fn checkpoint_digest(&self) -> &ContentDigest {
        &self.checkpoint_digest
    }
}

/// Domain-owned metadata persistence boundary. Implementations may persist
/// and atomically compare snapshots, but cannot invent semantic transitions.
///
/// This low-level boundary grants no live Registry, effect, daemon,
/// capability-owner, publication, or delete authority. A composition root must
/// obtain those owners' current evidence before producing the pure transition;
/// callers cannot substitute an approval/currentness Boolean here.
pub trait ArtifactRepository {
    /// Loads one replay-verified current metadata snapshot.
    ///
    /// # Errors
    ///
    /// Returns a closed availability, corruption, or authority failure.
    fn load(
        &mut self,
        store_id: &ArtifactStoreIdentity,
    ) -> Result<Option<ArtifactRepositorySnapshot>, ArtifactRepositoryError>;

    /// Atomically replaces the exact expected current checkpoint with `next`.
    /// The returned snapshot is the replay-verified committed state.
    ///
    /// # Errors
    ///
    /// Returns stale-write, availability, serialization, ambiguity,
    /// corruption, authority, or pure-domain failure without implying commit.
    fn compare_and_swap(
        &mut self,
        expected_checkpoint_digest: &ContentDigest,
        next: &ArtifactRepositorySnapshot,
    ) -> Result<ArtifactRepositorySnapshot, ArtifactRepositoryError>;
}

pub(crate) fn repository_canonical_bytes(
    value: &CanonicalValue,
) -> Result<Vec<u8>, ArtifactStoreReplayError> {
    canonicalize(value).map_err(|_| ArtifactStoreReplayError::Canonicalization)?;
    let mut output = String::new();
    write_repository_value(value, &mut output);
    Ok(output.into_bytes())
}

pub(crate) fn parse_canonical_bytes(
    bytes: &[u8],
    maximum: usize,
) -> Result<CanonicalValue, ArtifactStoreReplayError> {
    if bytes.is_empty() || bytes.len() > maximum {
        return Err(ArtifactStoreReplayError::ReplayLimit {
            field: "repository_bytes",
        });
    }
    let text =
        std::str::from_utf8(bytes).map_err(|_| ArtifactStoreReplayError::Canonicalization)?;
    let value = CanonicalJsonParser::new(text)
        .parse()
        .map_err(|()| ArtifactStoreReplayError::Canonicalization)?;
    if repository_canonical_bytes(&value)?.as_slice() != bytes {
        return Err(ArtifactStoreReplayError::Canonicalization);
    }
    Ok(value)
}

fn write_repository_value(value: &CanonicalValue, output: &mut String) {
    match value {
        CanonicalValue::Null => output.push_str("null"),
        CanonicalValue::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
        CanonicalValue::String(value) => write_repository_string(&normalize_nfc(value), output),
        CanonicalValue::Array(values) => {
            output.push('[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                write_repository_value(value, output);
            }
            output.push(']');
        }
        CanonicalValue::Object(entries) => {
            output.push('{');
            for (index, (key, value)) in entries.iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                write_repository_string(&normalize_nfc(key), output);
                output.push(':');
                write_repository_value(value, output);
            }
            output.push('}');
        }
    }
}

fn write_repository_string(value: &str, output: &mut String) {
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

/// Minimal bounded parser for the canonical value model. Number tokens,
/// whitespace, trailing bytes, and alternate encodings are rejected.
struct CanonicalJsonParser<'a> {
    input: &'a str,
    position: usize,
}

impl<'a> CanonicalJsonParser<'a> {
    const fn new(input: &'a str) -> Self {
        Self { input, position: 0 }
    }

    fn parse(mut self) -> Result<CanonicalValue, ()> {
        let value = self.value(0)?;
        if self.position != self.input.len() {
            return Err(());
        }
        Ok(value)
    }

    fn value(&mut self, depth: usize) -> Result<CanonicalValue, ()> {
        if depth > MAX_CANONICAL_NESTING_DEPTH {
            return Err(());
        }
        match self.peek() {
            Some(b'n') => {
                self.literal("null")?;
                Ok(CanonicalValue::Null)
            }
            Some(b't') => {
                self.literal("true")?;
                Ok(CanonicalValue::Bool(true))
            }
            Some(b'f') => {
                self.literal("false")?;
                Ok(CanonicalValue::Bool(false))
            }
            Some(b'"') => self.string().map(CanonicalValue::String),
            Some(b'[') => self.array(depth),
            Some(b'{') => self.object(depth),
            _ => Err(()),
        }
    }

    fn array(&mut self, depth: usize) -> Result<CanonicalValue, ()> {
        self.byte(b'[')?;
        let mut values = Vec::new();
        if self.take(b']') {
            return Ok(CanonicalValue::Array(values));
        }
        loop {
            values.push(self.value(depth + 1)?);
            if self.take(b']') {
                break;
            }
            self.byte(b',')?;
        }
        Ok(CanonicalValue::Array(values))
    }

    fn object(&mut self, depth: usize) -> Result<CanonicalValue, ()> {
        self.byte(b'{')?;
        let mut entries = Vec::new();
        if self.take(b'}') {
            return Ok(CanonicalValue::Object(entries));
        }
        loop {
            let key = self.string()?;
            self.byte(b':')?;
            entries.push((key, self.value(depth + 1)?));
            if self.take(b'}') {
                break;
            }
            self.byte(b',')?;
        }
        Ok(CanonicalValue::Object(entries))
    }

    fn string(&mut self) -> Result<String, ()> {
        self.byte(b'"')?;
        let mut output = String::new();
        loop {
            let byte = self.peek().ok_or(())?;
            match byte {
                b'"' => {
                    self.position += 1;
                    return Ok(output);
                }
                b'\\' => {
                    self.position += 1;
                    self.escape(&mut output)?;
                }
                0x00..=0x1f => return Err(()),
                _ => {
                    let character = self.input[self.position..].chars().next().ok_or(())?;
                    output.push(character);
                    self.position += character.len_utf8();
                }
            }
        }
    }

    fn escape(&mut self, output: &mut String) -> Result<(), ()> {
        let escaped = self.peek().ok_or(())?;
        self.position += 1;
        match escaped {
            b'"' => output.push('"'),
            b'\\' => output.push('\\'),
            b'/' => output.push('/'),
            b'b' => output.push('\u{8}'),
            b'f' => output.push('\u{c}'),
            b'n' => output.push('\n'),
            b'r' => output.push('\r'),
            b't' => output.push('\t'),
            b'u' => self.unicode_escape(output)?,
            _ => return Err(()),
        }
        Ok(())
    }

    fn unicode_escape(&mut self, output: &mut String) -> Result<(), ()> {
        let first = self.hex_quad()?;
        let scalar = if (0xd800..=0xdbff).contains(&first) {
            self.byte(b'\\')?;
            self.byte(b'u')?;
            let second = self.hex_quad()?;
            if !(0xdc00..=0xdfff).contains(&second) {
                return Err(());
            }
            0x1_0000 + ((u32::from(first) - 0xd800) << 10) + (u32::from(second) - 0xdc00)
        } else if (0xdc00..=0xdfff).contains(&first) {
            return Err(());
        } else {
            u32::from(first)
        };
        output.push(char::from_u32(scalar).ok_or(())?);
        Ok(())
    }

    fn hex_quad(&mut self) -> Result<u16, ()> {
        let mut value = 0_u16;
        for _ in 0..4 {
            let digit = match self.peek().ok_or(())? {
                b'0'..=b'9' => u16::from(self.input.as_bytes()[self.position] - b'0'),
                b'a'..=b'f' => u16::from(self.input.as_bytes()[self.position] - b'a' + 10),
                b'A'..=b'F' => u16::from(self.input.as_bytes()[self.position] - b'A' + 10),
                _ => return Err(()),
            };
            self.position += 1;
            value = value
                .checked_mul(16)
                .and_then(|v| v.checked_add(digit))
                .ok_or(())?;
        }
        Ok(value)
    }

    fn literal(&mut self, value: &str) -> Result<(), ()> {
        if self.input[self.position..].starts_with(value) {
            self.position += value.len();
            Ok(())
        } else {
            Err(())
        }
    }

    fn byte(&mut self, expected: u8) -> Result<(), ()> {
        if self.take(expected) { Ok(()) } else { Err(()) }
    }

    fn take(&mut self, expected: u8) -> bool {
        if self.peek() == Some(expected) {
            self.position += 1;
            true
        } else {
            false
        }
    }

    fn peek(&self) -> Option<u8> {
        self.input.as_bytes().get(self.position).copied()
    }
}
