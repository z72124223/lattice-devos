//! Host-owned Registry epoch anchor, independent of untrusted database contents.
//!
//! The root and database identity must come from trusted host configuration, never
//! a database row or purge plan. This protects against database-only cross-epoch
//! replacement. It does not protect against an OS user rewriting both stores, or
//! same-epoch tail rollback. `sync_all` plus rename is not a power-loss guarantee.
//! Missing anchors and pending transitions require the caller's fail-closed DB
//! reconciliation; this module never infers a seal from the database.

use std::fmt;
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub const ANCHOR_SCHEMA: &str = "lattice.registry-epoch.anchor.v1";
pub const ANCHOR_FILE: &str = "registry-epoch.anchor.json";
pub const ANCHOR_LOCK: &str = ".lock";
pub const MAX_ANCHOR_BYTES: u64 = 64 * 1024;

/// Identity obtained from the actual open handle, not a second path lookup.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AnchorFileIdentity {
    pub device: u64,
    pub inode: u64,
    pub links: u64,
}

/// Windows hosts must install their native, same-handle identity audit once.
/// The callback must return the actual volume/file identity and hard-link count.
pub type AnchorFileAudit = fn(&File) -> io::Result<AnchorFileIdentity>;
static FILE_AUDIT: OnceLock<AnchorFileAudit> = OnceLock::new();
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// # Errors
/// Fails if an audit callback was already installed.
pub fn install_anchor_file_audit(audit: AnchorFileAudit) -> AnchorResult<()> {
    FILE_AUDIT
        .set(audit)
        .map_err(|_| AnchorError::AuditAlreadyInstalled)
}

#[derive(Debug)]
pub enum AnchorError {
    InvalidDigest,
    InvalidEpoch,
    InvalidPath,
    PathAlias,
    HardLink,
    AuditUnavailable,
    AuditAlreadyInstalled,
    InvalidDocument,
    WrongDatabase,
    TooLarge,
    Locked,
    StateMismatch,
    Io(io::Error),
}

pub type AnchorResult<T> = Result<T, AnchorError>;

impl fmt::Display for AnchorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let code = match self {
            Self::InvalidDigest => "REGISTRY_ANCHOR_DIGEST_INVALID",
            Self::InvalidEpoch => "REGISTRY_ANCHOR_EPOCH_INVALID",
            Self::InvalidPath => "REGISTRY_ANCHOR_PATH_INVALID",
            Self::PathAlias => "REGISTRY_ANCHOR_PATH_ALIAS",
            Self::HardLink => "REGISTRY_ANCHOR_HARDLINK_REJECTED",
            Self::AuditUnavailable => "REGISTRY_ANCHOR_FILE_AUDIT_REQUIRED",
            Self::AuditAlreadyInstalled => "REGISTRY_ANCHOR_FILE_AUDIT_ALREADY_INSTALLED",
            Self::InvalidDocument => "REGISTRY_ANCHOR_DOCUMENT_INVALID",
            Self::WrongDatabase => "REGISTRY_ANCHOR_DATABASE_MISMATCH",
            Self::TooLarge => "REGISTRY_ANCHOR_TOO_LARGE",
            Self::Locked => "REGISTRY_ANCHOR_LOCKED",
            Self::StateMismatch => "REGISTRY_ANCHOR_STATE_MISMATCH",
            Self::Io(_) => "REGISTRY_ANCHOR_IO_FAILED",
        };
        f.write_str(code)
    }
}

impl std::error::Error for AnchorError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for AnchorError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnchorDigest(String);

impl AnchorDigest {
    /// # Errors
    /// Rejects zero or malformed SHA-256 digests.
    pub fn new(value: &str) -> AnchorResult<Self> {
        if value.len() != 64
            || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
            || value.bytes().all(|byte| byte == b'0')
        {
            return Err(AnchorError::InvalidDigest);
        }
        Ok(Self(value.to_ascii_lowercase()))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActiveAnchor {
    pub epoch: u64,
    pub seal_digest: AnchorDigest,
}

impl ActiveAnchor {
    /// # Errors
    /// Rejects epoch zero.
    pub fn new(epoch: u64, seal_digest: AnchorDigest) -> AnchorResult<Self> {
        if epoch == 0 {
            return Err(AnchorError::InvalidEpoch);
        }
        Ok(Self { epoch, seal_digest })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingAnchor {
    pub previous: Option<ActiveAnchor>,
    pub next: ActiveAnchor,
    pub operation_digest: AnchorDigest,
}

impl PendingAnchor {
    fn validate(&self) -> AnchorResult<()> {
        let expected = match &self.previous {
            Some(previous) if previous.epoch > 0 => previous.epoch.checked_add(1),
            Some(_) => None,
            None => Some(1),
        };
        if expected != Some(self.next.epoch) {
            return Err(AnchorError::InvalidEpoch);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AnchorState {
    Absent,
    Active(ActiveAnchor),
    Pending(PendingAnchor),
}

#[derive(Clone, Debug)]
pub struct RegistryEpochAnchor {
    root: PathBuf,
    database_identity: AnchorDigest,
}

impl RegistryEpochAnchor {
    /// Construct from trusted host configuration. This performs no writes.
    /// # Errors
    /// Rejects non-absolute paths and invalid database digests.
    pub fn new(root: PathBuf, database_identity_digest: &str) -> AnchorResult<Self> {
        validate_absolute(&root)?;
        Ok(Self {
            root,
            database_identity: AnchorDigest::new(database_identity_digest)?,
        })
    }

    /// No mkdir, lock creation, recovery, or stale-lock removal occurs here.
    /// # Errors
    /// Rejects aliases, locks, malformed records and failed file reads.
    pub fn read(&self) -> AnchorResult<AnchorState> {
        if !inspect_existing(&self.root, true)? {
            return Ok(AnchorState::Absent);
        }
        if inspect_existing(&self.root.join(ANCHOR_LOCK), false)? {
            open_regular(&self.root.join(ANCHOR_LOCK))?;
            return Err(AnchorError::Locked);
        }
        let state = self.read_unlocked()?;
        // A concurrent transition must not silently appear to be a stable read.
        if inspect_existing(&self.root.join(ANCHOR_LOCK), false)? {
            return Err(AnchorError::Locked);
        }
        Ok(state)
    }

    /// Explicitly create the root only. Never bootstrap an active seal from DB.
    /// # Errors
    /// Rejects unsafe paths, existing locks and failed file operations.
    pub fn initialize(&self) -> AnchorResult<AnchorState> {
        let _lock = self.lock(true)?;
        self.read_unlocked()
    }

    /// Exact compare-and-prepare. Repeating the identical pending operation is
    /// idempotent; another pending operation or mismatched previous state fails.
    /// # Errors
    /// Rejects unsafe paths, locked or mismatched state and failed writes.
    pub fn prepare(
        &self,
        expected_previous: Option<&ActiveAnchor>,
        next: &ActiveAnchor,
        operation_digest: &AnchorDigest,
    ) -> AnchorResult<PendingAnchor> {
        let pending = PendingAnchor {
            previous: expected_previous.cloned(),
            next: next.clone(),
            operation_digest: operation_digest.clone(),
        };
        pending.validate()?;
        let _lock = self.lock(true)?;
        match self.read_unlocked()? {
            AnchorState::Pending(current) if current == pending => return Ok(current),
            AnchorState::Absent if expected_previous.is_none() => {}
            AnchorState::Active(current) if Some(&current) == expected_previous => {}
            _ => return Err(AnchorError::StateMismatch),
        }
        self.write(&AnchorState::Pending(pending.clone()))?;
        Ok(pending)
    }

    /// The caller must independently match the committed DB receipt before this
    /// step. No database value is read or accepted as a replacement seal here.
    /// # Errors
    /// Rejects unsafe paths, locked or mismatched pending state and failed writes.
    pub fn activate(&self, expected_pending: &PendingAnchor) -> AnchorResult<ActiveAnchor> {
        expected_pending.validate()?;
        let _lock = self.lock(false)?;
        if self.read_unlocked()? != AnchorState::Pending(expected_pending.clone()) {
            return Err(AnchorError::StateMismatch);
        }
        self.write(&AnchorState::Active(expected_pending.next.clone()))?;
        Ok(expected_pending.next.clone())
    }

    fn read_unlocked(&self) -> AnchorResult<AnchorState> {
        let path = self.root.join(ANCHOR_FILE);
        if !inspect_existing(&path, false)? {
            return Ok(AnchorState::Absent);
        }
        let (file, _) = open_regular(&path)?;
        decode(&read_bounded(&file)?, &self.database_identity)
    }

    fn lock(&self, create_root: bool) -> AnchorResult<OwnedLock> {
        require_file_audit()?;
        if !inspect_existing(&self.root, true)? {
            if !create_root {
                return Err(AnchorError::StateMismatch);
            }
            fs::create_dir_all(&self.root)?;
            inspect_existing(&self.root, true)?;
        }
        let path = self.root.join(ANCHOR_LOCK);
        if inspect_existing(&path, false)? {
            open_regular(&path)?;
            return Err(AnchorError::Locked);
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create_new(true);
        no_follow(&mut options);
        let file = options.open(&path).map_err(|error| {
            if error.kind() == io::ErrorKind::AlreadyExists {
                AnchorError::Locked
            } else {
                AnchorError::Io(error)
            }
        })?;
        let identity = audit_file(&file)?;
        let token = unique_token();
        let mut lock = OwnedLock {
            path,
            file,
            identity,
            token,
        };
        lock.file.write_all(lock.token.as_bytes())?;
        lock.file.sync_all()?;
        Ok(lock)
    }

    fn write(&self, state: &AnchorState) -> AnchorResult<()> {
        let body = encode(state, &self.database_identity)?;
        if body.len() as u64 > MAX_ANCHOR_BYTES {
            return Err(AnchorError::TooLarge);
        }
        let path = self.root.join(ANCHOR_FILE);
        inspect_existing(&path, false)?;
        let temporary = self
            .root
            .join(format!(".registry-epoch.{}.tmp", unique_token()));
        let mut options = OpenOptions::new();
        options.read(true).write(true).create_new(true);
        no_follow(&mut options);
        let mut file = options.open(&temporary)?;
        audit_file(&file)?;
        file.write_all(&body)?;
        file.sync_all()?;
        audit_file(&file)?;
        drop(file);
        inspect_existing(&temporary, false)?;
        fs::rename(&temporary, &path)?;
        if self.read_unlocked()? != *state {
            return Err(AnchorError::StateMismatch);
        }
        Ok(())
    }
}

struct OwnedLock {
    path: PathBuf,
    file: File,
    identity: AnchorFileIdentity,
    token: String,
}

impl Drop for OwnedLock {
    fn drop(&mut self) {
        // Release only this acquired lock. A stale or replaced lock is never
        // removed. Failure to prove ownership leaves the file in place.
        if let Ok((file, identity)) = open_regular(&self.path)
            && identity == self.identity
            && read_bounded(&file).is_ok_and(|body| body == self.token.as_bytes())
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn unique_token() -> String {
    let counter = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let bytes = Sha256::digest(format!("{}:{counter}:{time}", std::process::id()).as_bytes());
    let hex = b"0123456789abcdef";
    let mut output = String::with_capacity(64);
    for byte in bytes {
        output.push(char::from(hex[usize::from(byte >> 4)]));
        output.push(char::from(hex[usize::from(byte & 15)]));
    }
    output
}

fn validate_absolute(path: &Path) -> AnchorResult<()> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
    {
        return Err(AnchorError::InvalidPath);
    }
    Ok(())
}

fn inspect_existing(path: &Path, directory: bool) -> AnchorResult<bool> {
    validate_absolute(path)?;
    let ancestors: Vec<_> = path.ancestors().collect();
    for part in ancestors.into_iter().rev() {
        let metadata = match fs::symlink_metadata(part) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error.into()),
        };
        reject_redirect(&metadata)?;
        if (part != path || directory) && !metadata.is_dir()
            || part == path && !directory && !metadata.is_file()
        {
            return Err(AnchorError::InvalidPath);
        }
        if !same_canonical_path(part, &fs::canonicalize(part)?) {
            return Err(AnchorError::PathAlias);
        }
    }
    Ok(true)
}

fn reject_redirect(metadata: &Metadata) -> AnchorResult<()> {
    if metadata.file_type().is_symlink() {
        return Err(AnchorError::PathAlias);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(AnchorError::PathAlias);
        }
    }
    Ok(())
}

#[cfg(windows)]
fn same_canonical_path(left: &Path, right: &Path) -> bool {
    fn normalize(path: &Path) -> Option<String> {
        let value = path.to_str()?.replace('/', "\\");
        if let Some(rest) = value.strip_prefix("\\\\?\\UNC\\") {
            Some(format!("\\\\{rest}"))
        } else {
            Some(value.strip_prefix("\\\\?\\").unwrap_or(&value).to_owned())
        }
    }
    match (normalize(left), normalize(right)) {
        (Some(left), Some(right)) => left.eq_ignore_ascii_case(&right),
        _ => false,
    }
}

#[cfg(not(windows))]
fn same_canonical_path(left: &Path, right: &Path) -> bool {
    left.as_os_str() == right.as_os_str()
}

fn no_follow(options: &mut OpenOptions) {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // OPEN_REPARSE_POINT, SHARE_READ | SHARE_DELETE. No writable sharing.
        options.custom_flags(0x0020_0000).share_mode(0x1 | 0x4);
    }
    #[cfg(not(windows))]
    let _ = options;
}

fn require_file_audit() -> AnchorResult<()> {
    #[cfg(not(unix))]
    if FILE_AUDIT.get().is_none() {
        return Err(AnchorError::AuditUnavailable);
    }
    Ok(())
}

fn audit_file(file: &File) -> AnchorResult<AnchorFileIdentity> {
    let metadata = file.metadata()?;
    reject_redirect(&metadata)?;
    if !metadata.is_file() {
        return Err(AnchorError::InvalidPath);
    }
    #[cfg(unix)]
    let identity = {
        use std::os::unix::fs::MetadataExt;
        AnchorFileIdentity {
            device: metadata.dev(),
            inode: metadata.ino(),
            links: metadata.nlink(),
        }
    };
    #[cfg(not(unix))]
    let identity = FILE_AUDIT.get().ok_or(AnchorError::AuditUnavailable)?(file)?;
    if identity.links != 1 {
        return Err(AnchorError::HardLink);
    }
    Ok(identity)
}

fn open_regular(path: &Path) -> AnchorResult<(File, AnchorFileIdentity)> {
    if !inspect_existing(path, false)? {
        return Err(AnchorError::StateMismatch);
    }
    let mut options = OpenOptions::new();
    options.read(true);
    no_follow(&mut options);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // Readers must be able to inspect our own still-open write lock. The
        // lock writer itself disallows other writers through its share mode.
        options.share_mode(0x1 | 0x2 | 0x4);
    }
    let file = options.open(path)?;
    let identity = audit_file(&file)?;
    Ok((file, identity))
}

fn read_bounded(file: &File) -> AnchorResult<Vec<u8>> {
    if file.metadata()?.len() > MAX_ANCHOR_BYTES {
        return Err(AnchorError::TooLarge);
    }
    let mut body = Vec::new();
    file.take(MAX_ANCHOR_BYTES + 1).read_to_end(&mut body)?;
    if body.len() as u64 > MAX_ANCHOR_BYTES {
        return Err(AnchorError::TooLarge);
    }
    Ok(body)
}

fn active_json(active: &ActiveAnchor) -> Value {
    json!({"epoch": active.epoch, "sealDigest": active.seal_digest.as_str()})
}

fn encode(state: &AnchorState, database: &AnchorDigest) -> AnchorResult<Vec<u8>> {
    let value = match state {
        AnchorState::Absent => return Err(AnchorError::InvalidDocument),
        AnchorState::Active(active) if active.epoch > 0 => json!({
            "schema": ANCHOR_SCHEMA, "dbIdentityDigest": database.as_str(), "status": "active",
            "epoch": active.epoch, "sealDigest": active.seal_digest.as_str(),
        }),
        AnchorState::Active(_) => return Err(AnchorError::InvalidEpoch),
        AnchorState::Pending(pending) => {
            pending.validate()?;
            json!({"schema": ANCHOR_SCHEMA, "dbIdentityDigest": database.as_str(), "status": "pending",
                "previous": pending.previous.as_ref().map(active_json), "next": active_json(&pending.next),
                "operationDigest": pending.operation_digest.as_str()})
        }
    };
    serde_json::to_vec(&value).map_err(|_| AnchorError::InvalidDocument)
}

fn exact_fields(value: &Value, fields: &[&str]) -> AnchorResult<()> {
    if !value.as_object().is_some_and(|object| {
        object.len() == fields.len() && fields.iter().all(|field| object.contains_key(*field))
    }) {
        return Err(AnchorError::InvalidDocument);
    }
    Ok(())
}

fn text_field<'a>(value: &'a Value, field: &str) -> AnchorResult<&'a str> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or(AnchorError::InvalidDocument)
}

fn decode_active(value: &Value) -> AnchorResult<ActiveAnchor> {
    exact_fields(value, &["epoch", "sealDigest"])?;
    ActiveAnchor::new(
        value["epoch"].as_u64().ok_or(AnchorError::InvalidEpoch)?,
        AnchorDigest::new(text_field(value, "sealDigest")?)?,
    )
}

fn decode(body: &[u8], database: &AnchorDigest) -> AnchorResult<AnchorState> {
    if body.len() as u64 > MAX_ANCHOR_BYTES {
        return Err(AnchorError::TooLarge);
    }
    let value: Value = serde_json::from_slice(body).map_err(|_| AnchorError::InvalidDocument)?;
    // Our closed format uses exactly the canonical bytes emitted by encode.
    // This also rejects duplicate keys rather than accepting JSON's last value.
    if serde_json::to_vec(&value).map_err(|_| AnchorError::InvalidDocument)? != body
        || text_field(&value, "schema")? != ANCHOR_SCHEMA
    {
        return Err(AnchorError::InvalidDocument);
    }
    if AnchorDigest::new(text_field(&value, "dbIdentityDigest")?)? != *database {
        return Err(AnchorError::WrongDatabase);
    }
    let state = match text_field(&value, "status")? {
        "active" => {
            exact_fields(
                &value,
                &[
                    "schema",
                    "dbIdentityDigest",
                    "status",
                    "epoch",
                    "sealDigest",
                ],
            )?;
            AnchorState::Active(decode_active(
                &json!({"epoch": value["epoch"], "sealDigest": value["sealDigest"]}),
            )?)
        }
        "pending" => {
            exact_fields(
                &value,
                &[
                    "schema",
                    "dbIdentityDigest",
                    "status",
                    "previous",
                    "next",
                    "operationDigest",
                ],
            )?;
            let pending = PendingAnchor {
                previous: if value["previous"].is_null() {
                    None
                } else {
                    Some(decode_active(&value["previous"])?)
                },
                next: decode_active(&value["next"])?,
                operation_digest: AnchorDigest::new(text_field(&value, "operationDigest")?)?,
            };
            pending.validate()?;
            AnchorState::Pending(pending)
        }
        _ => return Err(AnchorError::InvalidDocument),
    };
    if encode(&state, database)? != body {
        return Err(AnchorError::InvalidDocument);
    }
    Ok(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Once;

    static INSTALL_TEST_AUDIT: Once = Once::new();

    // Explicitly synthetic Windows audit. Native Windows link counts are an
    // integration responsibility of the runtime host's real callback.
    fn synthetic_audit(file: &File) -> io::Result<AnchorFileIdentity> {
        Ok(AnchorFileIdentity {
            device: 0,
            inode: 0,
            links: if file.metadata()?.len() == 4097 { 2 } else { 1 },
        })
    }

    struct Fixture {
        root: PathBuf,
        anchor: RegistryEpochAnchor,
    }

    impl Fixture {
        fn new() -> Self {
            INSTALL_TEST_AUDIT.call_once(|| install_anchor_file_audit(synthetic_audit).unwrap());
            let root =
                std::env::temp_dir().join(format!("lattice-registry-anchor-{}", unique_token()));
            let anchor = RegistryEpochAnchor::new(root.clone(), &"a".repeat(64)).unwrap();
            Self { root, anchor }
        }

        fn first(&self) -> ActiveAnchor {
            let first = active(1, 'b');
            let pending = self.anchor.prepare(None, &first, &digest('d')).unwrap();
            self.anchor.activate(&pending).unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            if self.root.exists() {
                fs::remove_dir_all(&self.root).unwrap();
            }
        }
    }

    fn digest(character: char) -> AnchorDigest {
        AnchorDigest::new(&character.to_string().repeat(64)).unwrap()
    }

    fn active(epoch: u64, character: char) -> ActiveAnchor {
        ActiveAnchor::new(epoch, digest(character)).unwrap()
    }

    #[test]
    fn absent_read_does_not_create_root_or_lock() {
        let fixture = Fixture::new();
        assert_eq!(fixture.anchor.read().unwrap(), AnchorState::Absent);
        assert!(!fixture.root.exists());
        assert_eq!(fixture.anchor.initialize().unwrap(), AnchorState::Absent);
        assert!(fixture.root.is_dir());
        assert_eq!(fs::read_dir(&fixture.root).unwrap().count(), 0);
        assert!(matches!(
            fixture.anchor.activate(&PendingAnchor {
                previous: None,
                next: active(1, 'b'),
                operation_digest: digest('d'),
            }),
            Err(AnchorError::StateMismatch)
        ));
        assert!(!fixture.root.join(ANCHOR_FILE).exists());
    }

    #[test]
    fn exact_epoch_one_to_two_and_pending_resume() {
        let fixture = Fixture::new();
        let first = fixture.first();
        let reopened = RegistryEpochAnchor::new(fixture.root.clone(), &"a".repeat(64)).unwrap();
        let pending = reopened
            .prepare(Some(&first), &active(2, 'c'), &digest('e'))
            .unwrap();
        assert_eq!(
            fixture.anchor.read().unwrap(),
            AnchorState::Pending(pending.clone())
        );
        assert_eq!(
            reopened
                .prepare(Some(&first), &pending.next, &digest('e'))
                .unwrap(),
            pending
        );
        let mut wrong_operation = pending.clone();
        wrong_operation.operation_digest = digest('f');
        assert!(matches!(
            reopened.activate(&wrong_operation),
            Err(AnchorError::StateMismatch)
        ));
        assert_eq!(
            reopened.read().unwrap(),
            AnchorState::Pending(pending.clone())
        );
        assert_eq!(reopened.activate(&pending).unwrap(), active(2, 'c'));
        assert_eq!(
            reopened.read().unwrap(),
            AnchorState::Active(active(2, 'c'))
        );
        assert!(matches!(
            reopened.activate(&pending),
            Err(AnchorError::StateMismatch)
        ));
        assert!(!fixture.root.join(ANCHOR_LOCK).exists());
    }

    #[test]
    fn wrong_database_digest_epoch_skip_reuse_and_previous_fail_closed() {
        for value in [
            String::new(),
            "0".repeat(64),
            "x".repeat(64),
            "a".repeat(63),
        ] {
            assert!(matches!(
                AnchorDigest::new(&value),
                Err(AnchorError::InvalidDigest)
            ));
        }
        assert!(matches!(
            ActiveAnchor::new(0, digest('b')),
            Err(AnchorError::InvalidEpoch)
        ));
        let fixture = Fixture::new();
        let first = fixture.first();
        let bytes = fs::read(fixture.root.join(ANCHOR_FILE)).unwrap();
        let wrong_db = RegistryEpochAnchor::new(fixture.root.clone(), &"f".repeat(64)).unwrap();
        assert!(matches!(wrong_db.read(), Err(AnchorError::WrongDatabase)));
        for epoch in [1, 3] {
            assert!(matches!(
                fixture
                    .anchor
                    .prepare(Some(&first), &active(epoch, 'c'), &digest('e')),
                Err(AnchorError::InvalidEpoch)
            ));
        }
        assert!(matches!(
            fixture
                .anchor
                .prepare(Some(&active(1, 'f')), &active(2, 'c'), &digest('e')),
            Err(AnchorError::StateMismatch)
        ));
        assert!(matches!(
            fixture.anchor.prepare(None, &active(1, 'b'), &digest('e')),
            Err(AnchorError::StateMismatch)
        ));
        assert_eq!(fs::read(fixture.root.join(ANCHOR_FILE)).unwrap(), bytes);
    }

    #[test]
    fn concurrent_and_stale_locks_are_not_removed_or_broken() {
        let fixture = Fixture::new();
        let held = fixture.anchor.lock(true).unwrap();
        assert!(matches!(fixture.anchor.read(), Err(AnchorError::Locked)));
        assert!(matches!(
            fixture.anchor.prepare(None, &active(1, 'b'), &digest('d')),
            Err(AnchorError::Locked)
        ));
        drop(held);
        assert!(!fixture.root.join(ANCHOR_LOCK).exists());
        fs::write(fixture.root.join(ANCHOR_LOCK), b"stale-lock-must-remain").unwrap();
        assert!(matches!(
            fixture.anchor.initialize(),
            Err(AnchorError::Locked)
        ));
        assert_eq!(
            fs::read(fixture.root.join(ANCHOR_LOCK)).unwrap(),
            b"stale-lock-must-remain"
        );
    }

    #[test]
    fn closed_bounded_document_rejects_unknown_duplicate_and_invalid_fields() {
        let fixture = Fixture::new();
        fixture.first();
        let path = fixture.root.join(ANCHOR_FILE);
        let original = fs::read(&path).unwrap();
        let mut value: Value = serde_json::from_slice(&original).unwrap();
        value["unexpected"] = json!(true);
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(matches!(
            fixture.anchor.read(),
            Err(AnchorError::InvalidDocument)
        ));
        let duplicate = format!(
            "{{\"status\":\"active\",{}",
            std::str::from_utf8(&original[1..]).unwrap()
        );
        fs::write(&path, duplicate).unwrap();
        assert!(matches!(
            fixture.anchor.read(),
            Err(AnchorError::InvalidDocument)
        ));
        fs::write(
            &path,
            vec![b' '; usize::try_from(MAX_ANCHOR_BYTES).unwrap() + 1],
        )
        .unwrap();
        assert!(matches!(fixture.anchor.read(), Err(AnchorError::TooLarge)));
        let mut value: Value = serde_json::from_slice(&original).unwrap();
        value["sealDigest"] = json!("0".repeat(64));
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(matches!(
            fixture.anchor.read(),
            Err(AnchorError::InvalidDigest)
        ));
    }

    #[test]
    fn hardlink_audit_rejection_is_not_treated_as_missing() {
        let fixture = Fixture::new();
        fixture.first();
        let anchor = fixture.root.join(ANCHOR_FILE);
        #[cfg(unix)]
        fs::hard_link(&anchor, fixture.root.join("another-name")).unwrap();
        #[cfg(not(unix))]
        fs::write(&anchor, vec![b'x'; 4097]).unwrap(); // Synthetic audit reports 2.
        assert!(matches!(fixture.anchor.read(), Err(AnchorError::HardLink)));
    }

    #[test]
    #[cfg(windows)]
    fn missing_audit_fails_closed_in_an_unconfigured_process() {
        const CHILD: &str = "LATTICE_ANCHOR_MISSING_AUDIT_UNIT_TEST";
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "registry_epoch_anchor::tests::missing_audit_fails_closed_in_an_unconfigured_process"])
                .env(CHILD, "1")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }
        assert!(FILE_AUDIT.get().is_none());
        let root = std::env::temp_dir().join(format!("lattice-anchor-no-audit-{}", unique_token()));
        let anchor = RegistryEpochAnchor::new(root.clone(), &"a".repeat(64)).unwrap();
        assert_eq!(anchor.read().unwrap(), AnchorState::Absent);
        assert!(matches!(
            anchor.prepare(None, &active(1, 'b'), &digest('d')),
            Err(AnchorError::AuditUnavailable)
        ));
        assert!(!root.exists());
        fs::create_dir(&root).unwrap();
        let bytes = encode(&AnchorState::Active(active(1, 'b')), &digest('a')).unwrap();
        fs::write(root.join(ANCHOR_FILE), &bytes).unwrap();
        assert!(matches!(anchor.read(), Err(AnchorError::AuditUnavailable)));
        assert_eq!(fs::read(root.join(ANCHOR_FILE)).unwrap(), bytes);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn relative_parent_and_symbolic_alias_paths_are_rejected() {
        let fixture = Fixture::new();
        assert!(matches!(
            RegistryEpochAnchor::new(PathBuf::from("relative"), &"a".repeat(64)),
            Err(AnchorError::InvalidPath)
        ));
        assert!(matches!(
            RegistryEpochAnchor::new(fixture.root.join("..").join("alias"), &"a".repeat(64)),
            Err(AnchorError::InvalidPath)
        ));
        fixture.anchor.initialize().unwrap();
        let link = fixture.root.join("alias");
        #[cfg(windows)]
        let linked = std::os::windows::fs::symlink_dir(&fixture.root, &link);
        #[cfg(unix)]
        let linked = std::os::unix::fs::symlink(&fixture.root, &link);
        #[cfg(not(any(unix, windows)))]
        let linked = Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "no symlink test",
        ));
        if linked.is_ok() {
            let alias = RegistryEpochAnchor::new(link, &"a".repeat(64)).unwrap();
            assert!(matches!(alias.read(), Err(AnchorError::PathAlias)));
        } else {
            eprintln!("native symlink creation unavailable; Windows alias test not verified");
        }
    }
}
