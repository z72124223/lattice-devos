//! Byte-only adapter for one physically verified disposable Artifact root.

use std::cell::RefCell;
use std::error::Error;
use std::fmt;
use std::fs::{self, File, Metadata, OpenOptions};
use std::hash::{Hash, Hasher};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

use lattice_contracts::ArtifactObjectIdentity;
use sha2::{Digest, Sha256};

/// Fixed marker filename required before root admission.
pub const OWNED_ROOT_MARKER_FILE: &str = ".lattice-artifact-owned-root";
/// Absolute streamed byte bound matching Artifact Store 1.1.
pub const MAX_OBJECT_BYTES: u64 = 1_073_741_824;
const STREAM_BUFFER_BYTES: usize = 64 * 1_024;
const MARKER_VERSION: &str = "lattice-artifact-owned-root/1.0";
const WINDOWS_REPARSE_POINT: u32 = 0x0000_0400;
const WINDOWS_DEVICE: u32 = 0x0000_0040;

/// Returns the exact marker bytes an external fixture owner must create and
/// durably flush before admission.
#[must_use]
pub fn owned_root_marker_bytes(root_id: &str) -> Vec<u8> {
    format!("{MARKER_VERSION}\nroot_id={root_id}\n").into_bytes()
}

/// Closed physical-adapter failure classification.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OwnedRootErrorKind {
    /// Root marker, physical identity, or fixed shape is missing/invalid.
    UnverifiedRoot,
    /// The root overlaps a registered product root after case folding.
    ProductRootOverlap,
    /// A symlink, junction, reparse point, device, hardlink, or non-regular
    /// object was observed.
    UnsafeFileKind,
    /// A derived internal path escaped or a caller path form was unsafe.
    Containment,
    /// Declared or streamed bytes exceeded the fixed bound.
    ByteLimit,
    /// Stream length differed from the exact declaration.
    LengthMismatch,
    /// Stream or stored bytes differed from the object digest.
    DigestMismatch,
    /// The exact authoritative object file is absent.
    MissingObject,
    /// The durable delete-claim token was empty or malformed.
    InvalidClaim,
    /// A physical effect may have occurred and requires reconciliation.
    ReconciliationRequired,
    /// A verified no-effect filesystem operation failed.
    Io,
}

/// Redacted byte-adapter failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OwnedRootError {
    kind: OwnedRootErrorKind,
}

impl OwnedRootError {
    const fn new(kind: OwnedRootErrorKind) -> Self {
        Self { kind }
    }

    /// Returns the closed failure kind.
    #[must_use]
    pub const fn kind(self) -> OwnedRootErrorKind {
        self.kind
    }

    /// Returns a stable non-secret diagnostic code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self.kind {
            OwnedRootErrorKind::UnverifiedRoot => "UNVERIFIED_ROOT",
            OwnedRootErrorKind::ProductRootOverlap => "PRODUCT_ROOT_OVERLAP",
            OwnedRootErrorKind::UnsafeFileKind => "UNSAFE_FILE_KIND",
            OwnedRootErrorKind::Containment => "OWNED_ROOT_CONTAINMENT",
            OwnedRootErrorKind::ByteLimit => "ARTIFACT_BYTE_LIMIT",
            OwnedRootErrorKind::LengthMismatch => "ARTIFACT_LENGTH_MISMATCH",
            OwnedRootErrorKind::DigestMismatch => "ARTIFACT_DIGEST_MISMATCH",
            OwnedRootErrorKind::MissingObject => "ARTIFACT_OBJECT_MISSING",
            OwnedRootErrorKind::InvalidClaim => "ARTIFACT_DELETE_CLAIM_INVALID",
            OwnedRootErrorKind::ReconciliationRequired => {
                "ARTIFACT_FILESYSTEM_RECONCILIATION_REQUIRED"
            }
            OwnedRootErrorKind::Io => "ARTIFACT_FILESYSTEM_IO",
        }
    }
}

impl fmt::Display for OwnedRootError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl Error for OwnedRootError {}

/// Result of atomic no-clobber publication.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PublishDisposition {
    /// This call installed the first exact final object file.
    Published,
    /// Another publisher won and its exact bytes were verified before reuse.
    ReusedVerifiedWinner,
}

#[derive(Clone, Eq, PartialEq)]
struct PhysicalIdentity(Vec<u8>);

impl fmt::Debug for PhysicalIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[PHYSICAL_FILE_IDENTITY]")
    }
}

/// Opaque sealed staging capability. No path accessor exists.
pub struct SealedArtifact {
    root_identity: PhysicalIdentity,
    object: ArtifactObjectIdentity,
    declared_length: u64,
    file: tempfile::NamedTempFile,
    file_identity: PhysicalIdentity,
}

impl fmt::Debug for SealedArtifact {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SealedArtifact")
            .field("object", &self.object)
            .field("declared_length", &self.declared_length)
            .field("path", &"[OWNED_INTERNAL_PATH]")
            .finish_non_exhaustive()
    }
}

/// Opaque quarantined staging capability. It cannot be promoted by scanning.
pub struct QuarantinedArtifact {
    root_identity: PhysicalIdentity,
    path: PathBuf,
    file_identity: PhysicalIdentity,
}

impl QuarantinedArtifact {
    /// Confirms this value represents a quarantined exact file capability.
    #[must_use]
    pub const fn is_quarantined(&self) -> bool {
        true
    }
}

impl fmt::Debug for QuarantinedArtifact {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("QuarantinedArtifact")
            .field("path", &"[OWNED_INTERNAL_PATH]")
            .finish_non_exhaustive()
    }
}

/// Opaque admitted root capability. All later operation targets are derived
/// internally from typed Artifact identity.
pub struct OwnedArtifactRoot {
    root: PathBuf,
    root_id: String,
    root_identity: PhysicalIdentity,
    marker_identity: PhysicalIdentity,
    created_directories: RefCell<Vec<PathBuf>>,
}

impl fmt::Debug for OwnedArtifactRoot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OwnedArtifactRoot")
            .field("root_id", &self.root_id)
            .field("root", &"[VERIFIED_OWNED_ROOT]")
            .finish_non_exhaustive()
    }
}

impl OwnedArtifactRoot {
    /// Admits an existing disposable root after exact marker, physical
    /// identity, path-kind, and registered product-root separation checks.
    /// No filesystem object is created or changed during admission.
    ///
    /// # Errors
    ///
    /// Rejects relative/device/ADS paths, malformed root identity, absent or
    /// linked marker, reparse roots, and ancestor/descendant product overlap.
    pub fn admit(
        root: &Path,
        expected_root_id: &str,
        registered_product_roots: &[PathBuf],
    ) -> Result<Self, OwnedRootError> {
        validate_root_id(expected_root_id)?;
        reject_unsafe_caller_root(root)?;
        verify_caller_directory_chain(root)?;
        let canonical = fs::canonicalize(root).map_err(|_| unverified())?;
        let metadata = fs::symlink_metadata(&canonical).map_err(|_| unverified())?;
        verify_directory_metadata(&metadata)?;
        verify_directory_no_alternate_streams(&canonical)?;
        let root_identity = physical_identity_path(&canonical)?;
        let folded = folded_path(&canonical);
        for product in registered_product_roots {
            reject_unsafe_caller_root(product)?;
            let product = fs::canonicalize(product).map_err(|_| unverified())?;
            let product_folded = folded_path(&product);
            if path_prefix(&folded, &product_folded) || path_prefix(&product_folded, &folded) {
                return Err(OwnedRootError::new(OwnedRootErrorKind::ProductRootOverlap));
            }
        }
        let marker_identity = verify_marker(&canonical, expected_root_id)?;
        Ok(Self {
            root: canonical,
            root_id: expected_root_id.to_owned(),
            root_identity,
            marker_identity,
            created_directories: RefCell::new(Vec::new()),
        })
    }

    /// Streams one exact object into an exclusive same-root staging file,
    /// incrementally enforcing length/digest bounds and flushing the seal.
    ///
    /// # Errors
    ///
    /// Rejects root drift, oversized/incomplete/corrupt streams, unsafe file
    /// kinds, containment failure, or ambiguous cleanup.
    pub fn stage<R: Read>(
        &mut self,
        object: &ArtifactObjectIdentity,
        declared_length: u64,
        mut source: R,
    ) -> Result<SealedArtifact, OwnedRootError> {
        if declared_length > MAX_OBJECT_BYTES {
            return Err(OwnedRootError::new(OwnedRootErrorKind::ByteLimit));
        }
        self.verify_root()?;
        let staging_directory = self.ensure_directory(".staging")?;
        let mut file = tempfile::Builder::new()
            .prefix("stage-")
            .suffix(".owned")
            .tempfile_in(&staging_directory)
            .map_err(|_| OwnedRootError::new(OwnedRootErrorKind::Io))?;
        let path = file.path().to_path_buf();
        let mut digest = Sha256::new();
        let mut observed = 0_u64;
        let mut buffer = vec![0_u8; STREAM_BUFFER_BYTES].into_boxed_slice();
        let result = (|| {
            loop {
                let read = source
                    .read(&mut buffer)
                    .map_err(|_| OwnedRootError::new(OwnedRootErrorKind::Io))?;
                if read == 0 {
                    break;
                }
                observed = observed
                    .checked_add(u64::try_from(read).map_err(|_| byte_limit())?)
                    .ok_or_else(byte_limit)?;
                if observed > declared_length || observed > MAX_OBJECT_BYTES {
                    return Err(if observed > MAX_OBJECT_BYTES {
                        byte_limit()
                    } else {
                        OwnedRootError::new(OwnedRootErrorKind::LengthMismatch)
                    });
                }
                file.write_all(&buffer[..read])
                    .map_err(|_| OwnedRootError::new(OwnedRootErrorKind::Io))?;
                digest.update(&buffer[..read]);
            }
            if observed != declared_length {
                return Err(OwnedRootError::new(OwnedRootErrorKind::LengthMismatch));
            }
            if digest_hex(digest.finalize().as_slice()) != object.key().content_digest().as_str() {
                return Err(OwnedRootError::new(OwnedRootErrorKind::DigestMismatch));
            }
            file.as_file()
                .sync_all()
                .map_err(|_| OwnedRootError::new(OwnedRootErrorKind::Io))?;
            let metadata = file
                .as_file()
                .metadata()
                .map_err(|_| OwnedRootError::new(OwnedRootErrorKind::Io))?;
            verify_regular_single_link(&path, &metadata)?;
            if metadata.len() != declared_length {
                return Err(OwnedRootError::new(OwnedRootErrorKind::LengthMismatch));
            }
            let file_identity = physical_identity_file(file.as_file())?;
            sync_directory(&staging_directory)?;
            self.verify_root()?;
            Ok(file_identity)
        })();
        match result {
            Ok(file_identity) => Ok(SealedArtifact {
                root_identity: self.root_identity.clone(),
                object: object.clone(),
                declared_length,
                file,
                file_identity,
            }),
            Err(error) => {
                drop(file);
                if path.exists() {
                    return Err(reconciliation());
                }
                Err(error)
            }
        }
    }

    /// Atomically publishes a sealed staging capability without overwriting.
    /// A concurrent loser verifies the exact winner before returning reuse.
    ///
    /// # Errors
    ///
    /// Rejects capability/root drift, unsafe files, corrupt winners, or an
    /// ambiguous link/unlink result.
    #[allow(clippy::needless_pass_by_value)]
    pub fn publish(
        &mut self,
        sealed: SealedArtifact,
    ) -> Result<PublishDisposition, OwnedRootError> {
        self.verify_sealed(&sealed)?;
        let final_path = self.object_path(&sealed.object, true)?;
        let final_parent = final_path.parent().ok_or_else(containment)?.to_path_buf();
        let staging_parent = sealed
            .file
            .path()
            .parent()
            .ok_or_else(containment)?
            .to_path_buf();
        let SealedArtifact {
            object,
            declared_length,
            file,
            ..
        } = sealed;
        match file.persist_noclobber(&final_path) {
            Ok(persisted) => {
                persisted.sync_all().map_err(|_| reconciliation())?;
                sync_directory(&final_parent).map_err(|_| reconciliation())?;
                sync_directory(&staging_parent).map_err(|_| reconciliation())?;
                self.verify_object_file(&final_path, &object, declared_length)
                    .map_err(|_| reconciliation())?;
                self.verify_root().map_err(|_| reconciliation())?;
                Ok(PublishDisposition::Published)
            }
            Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => {
                let staging_path = error.file.path().to_path_buf();
                let winner = self.verify_object_file(&final_path, &object, declared_length);
                drop(error.file);
                if staging_path.exists() {
                    return Err(reconciliation());
                }
                winner?;
                sync_directory(&staging_parent)?;
                self.verify_root()?;
                Ok(PublishDisposition::ReusedVerifiedWinner)
            }
            Err(_) => Err(reconciliation()),
        }
    }

    /// Moves one exact sealed capability to an owner-controlled quarantine
    /// name. It does not inspect or promote directory contents.
    ///
    /// # Errors
    ///
    /// Rejects root/capability drift or ambiguous physical outcomes.
    #[allow(clippy::needless_pass_by_value)]
    pub fn quarantine(
        &mut self,
        sealed: SealedArtifact,
    ) -> Result<QuarantinedArtifact, OwnedRootError> {
        self.verify_sealed(&sealed)?;
        let directory = self.ensure_directory(".quarantine")?;
        let destination = self.unused_internal_path(&directory, "orphan")?;
        let staging_parent = sealed
            .file
            .path()
            .parent()
            .ok_or_else(containment)?
            .to_path_buf();
        let persisted = sealed
            .file
            .persist_noclobber(&destination)
            .map_err(|_| reconciliation())?;
        persisted.sync_all().map_err(|_| reconciliation())?;
        sync_directory(&directory).map_err(|_| reconciliation())?;
        sync_directory(&staging_parent).map_err(|_| reconciliation())?;
        let metadata = fs::symlink_metadata(&destination).map_err(|_| reconciliation())?;
        verify_regular_single_link(&destination, &metadata).map_err(|_| reconciliation())?;
        let file_identity = physical_identity_file(&persisted).map_err(|_| reconciliation())?;
        self.verify_root().map_err(|_| reconciliation())?;
        Ok(QuarantinedArtifact {
            root_identity: self.root_identity.clone(),
            path: destination,
            file_identity,
        })
    }

    /// Removes one exact quarantined capability. No directory scan is used.
    ///
    /// # Errors
    ///
    /// Rejects root, identity, link, or containment drift and ambiguous unlink.
    #[allow(clippy::needless_pass_by_value)]
    pub fn discard_quarantined(
        &mut self,
        quarantined: QuarantinedArtifact,
    ) -> Result<(), OwnedRootError> {
        self.verify_root()?;
        if quarantined.root_identity != self.root_identity {
            return Err(unverified());
        }
        self.verify_internal_file(&quarantined.path, &quarantined.file_identity)?;
        fs::remove_file(&quarantined.path).map_err(|_| reconciliation())?;
        if quarantined.path.exists() {
            return Err(reconciliation());
        }
        sync_directory(quarantined.path.parent().ok_or_else(containment)?)
            .map_err(|_| reconciliation())?;
        self.verify_root().map_err(|_| reconciliation())
    }

    /// Reads one exact derived object, rechecking regular-file identity,
    /// single-link shape, declared length and content digest.
    ///
    /// # Errors
    ///
    /// Rejects absent, unsafe, oversized, substituted, or corrupt bytes.
    pub fn read_verified(
        &self,
        object: &ArtifactObjectIdentity,
        declared_length: u64,
    ) -> Result<Vec<u8>, OwnedRootError> {
        if declared_length > MAX_OBJECT_BYTES {
            return Err(byte_limit());
        }
        self.verify_root()?;
        let path = self.object_path(object, false)?;
        let before = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(OwnedRootError::new(OwnedRootErrorKind::MissingObject));
            }
            Err(_) => return Err(OwnedRootError::new(OwnedRootErrorKind::Io)),
        };
        verify_regular_single_link(&path, &before)?;
        if before.len() != declared_length {
            return Err(OwnedRootError::new(OwnedRootErrorKind::LengthMismatch));
        }
        let before_identity = physical_identity_path(&path)?;
        let mut file =
            File::open(&path).map_err(|_| OwnedRootError::new(OwnedRootErrorKind::Io))?;
        let handle = file
            .metadata()
            .map_err(|_| OwnedRootError::new(OwnedRootErrorKind::Io))?;
        verify_regular_single_link(&path, &handle)?;
        if physical_identity_file(&file)? != before_identity {
            return Err(reconciliation());
        }
        let capacity = usize::try_from(declared_length).map_err(|_| byte_limit())?;
        let mut bytes = Vec::with_capacity(capacity);
        file.read_to_end(&mut bytes)
            .map_err(|_| OwnedRootError::new(OwnedRootErrorKind::Io))?;
        verify_bytes(object, declared_length, &bytes)?;
        let after = fs::symlink_metadata(&path).map_err(|_| reconciliation())?;
        verify_regular_single_link(&path, &after)?;
        if physical_identity_path(&path)? != before_identity {
            return Err(reconciliation());
        }
        self.verify_root()?;
        Ok(bytes)
    }

    /// Unlinks exactly one verified object after receiving a non-empty durable
    /// delete-claim token. The token is validated but never used as a path.
    ///
    /// # Errors
    ///
    /// Rejects malformed claims, absent/unsafe/substituted objects, root drift,
    /// or an ambiguous unlink outcome.
    pub fn unlink_claimed(
        &mut self,
        object: &ArtifactObjectIdentity,
        claim_token: &str,
    ) -> Result<(), OwnedRootError> {
        validate_claim_token(claim_token)?;
        self.verify_root()?;
        let path = self.object_path(object, false)?;
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(OwnedRootError::new(OwnedRootErrorKind::MissingObject));
            }
            Err(_) => return Err(OwnedRootError::new(OwnedRootErrorKind::Io)),
        };
        verify_regular_single_link(&path, &metadata)?;
        let identity = physical_identity_path(&path)?;
        self.verify_internal_file(&path, &identity)?;
        let deleting_directory = self.ensure_directory(".deleting")?;
        let deleting_path = self.unused_internal_path(&deleting_directory, "delete")?;
        fs::hard_link(&path, &deleting_path).map_err(|_| reconciliation())?;
        fs::remove_file(&path).map_err(|_| reconciliation())?;
        if path.exists() {
            return Err(reconciliation());
        }
        self.verify_internal_file(&deleting_path, &identity)
            .map_err(|_| reconciliation())?;
        fs::remove_file(&deleting_path).map_err(|_| reconciliation())?;
        if deleting_path.exists() {
            return Err(reconciliation());
        }
        sync_directory(path.parent().ok_or_else(containment)?).map_err(|_| reconciliation())?;
        sync_directory(&deleting_directory).map_err(|_| reconciliation())?;
        self.verify_root().map_err(|_| reconciliation())
    }

    /// Removes only adapter-created directories that are currently empty,
    /// one exact level at a time. It never removes files or traverses a tree.
    ///
    /// # Errors
    ///
    /// Rejects root/containment drift or a non-empty/unsafe created directory.
    pub fn cleanup_empty_fixture(&mut self) -> Result<(), OwnedRootError> {
        self.verify_root()?;
        let paths = {
            let mut paths = self.created_directories.borrow_mut();
            paths.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
            paths.drain(..).collect::<Vec<_>>()
        };
        for (index, path) in paths.iter().enumerate() {
            verify_contained(&self.root, path)?;
            match fs::remove_dir(path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(_) => {
                    self.created_directories
                        .borrow_mut()
                        .extend(paths[index..].iter().cloned());
                    return Err(OwnedRootError::new(OwnedRootErrorKind::Io));
                }
            }
        }
        self.verify_root()
    }

    fn verify_root(&self) -> Result<(), OwnedRootError> {
        let metadata = fs::symlink_metadata(&self.root).map_err(|_| unverified())?;
        verify_directory_metadata(&metadata)?;
        verify_directory_no_alternate_streams(&self.root)?;
        if physical_identity_path(&self.root)? != self.root_identity {
            return Err(unverified());
        }
        if verify_marker(&self.root, &self.root_id)? != self.marker_identity {
            return Err(unverified());
        }
        Ok(())
    }

    fn verify_sealed(&self, sealed: &SealedArtifact) -> Result<(), OwnedRootError> {
        self.verify_root()?;
        if sealed.root_identity != self.root_identity {
            return Err(unverified());
        }
        self.verify_internal_file(sealed.file.path(), &sealed.file_identity)?;
        if physical_identity_file(sealed.file.as_file())? != sealed.file_identity {
            return Err(reconciliation());
        }
        self.verify_object_file(sealed.file.path(), &sealed.object, sealed.declared_length)
    }

    fn verify_internal_file(
        &self,
        path: &Path,
        expected: &PhysicalIdentity,
    ) -> Result<(), OwnedRootError> {
        verify_contained(&self.root, path)?;
        verify_directory_chain(&self.root, path.parent().ok_or_else(containment)?)?;
        let metadata = fs::symlink_metadata(path).map_err(|_| reconciliation())?;
        verify_regular_single_link(path, &metadata)?;
        if &physical_identity_path(path)? != expected {
            return Err(reconciliation());
        }
        Ok(())
    }

    fn verify_object_file(
        &self,
        path: &Path,
        object: &ArtifactObjectIdentity,
        declared_length: u64,
    ) -> Result<(), OwnedRootError> {
        verify_contained(&self.root, path)?;
        let metadata = fs::symlink_metadata(path).map_err(|_| reconciliation())?;
        verify_regular_single_link(path, &metadata)?;
        if metadata.len() != declared_length {
            return Err(OwnedRootError::new(OwnedRootErrorKind::LengthMismatch));
        }
        let before_identity = physical_identity_path(path)?;
        let mut file = File::open(path).map_err(|_| OwnedRootError::new(OwnedRootErrorKind::Io))?;
        let handle_metadata = file
            .metadata()
            .map_err(|_| OwnedRootError::new(OwnedRootErrorKind::Io))?;
        verify_regular_single_link(path, &handle_metadata)?;
        if physical_identity_file(&file)? != before_identity {
            return Err(reconciliation());
        }
        let mut digest = Sha256::new();
        let mut length = 0_u64;
        let mut buffer = vec![0_u8; STREAM_BUFFER_BYTES].into_boxed_slice();
        loop {
            let read = file
                .read(&mut buffer)
                .map_err(|_| OwnedRootError::new(OwnedRootErrorKind::Io))?;
            if read == 0 {
                break;
            }
            length = length
                .checked_add(u64::try_from(read).map_err(|_| byte_limit())?)
                .ok_or_else(byte_limit)?;
            if length > declared_length || length > MAX_OBJECT_BYTES {
                return Err(OwnedRootError::new(OwnedRootErrorKind::LengthMismatch));
            }
            digest.update(&buffer[..read]);
        }
        if length != declared_length {
            return Err(OwnedRootError::new(OwnedRootErrorKind::LengthMismatch));
        }
        if digest_hex(digest.finalize().as_slice()) != object.key().content_digest().as_str() {
            return Err(OwnedRootError::new(OwnedRootErrorKind::DigestMismatch));
        }
        let after = fs::symlink_metadata(path).map_err(|_| reconciliation())?;
        verify_regular_single_link(path, &after)?;
        if physical_identity_path(path)? != before_identity {
            return Err(reconciliation());
        }
        Ok(())
    }

    fn object_path(
        &self,
        object: &ArtifactObjectIdentity,
        create_directories: bool,
    ) -> Result<PathBuf, OwnedRootError> {
        if object.key().algorithm() != "sha256" {
            return Err(containment());
        }
        let project = digest_hex(&Sha256::digest(
            object.key().project_id().as_str().as_bytes(),
        ));
        let digest = object.key().content_digest().as_str();
        if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(containment());
        }
        let components = [
            "objects".to_owned(),
            project,
            "sha256".to_owned(),
            digest[..2].to_owned(),
        ];
        let mut directory = self.root.clone();
        for component in components {
            directory.push(component);
            if create_directories {
                self.ensure_exact_directory(&directory)?;
            }
        }
        verify_directory_chain(&self.root, &directory)?;
        let path = directory.join(format!("{}-{}.blob", digest, object.generation().get()));
        verify_contained(&self.root, &path)?;
        Ok(path)
    }

    fn ensure_directory(&mut self, component: &str) -> Result<PathBuf, OwnedRootError> {
        let path = self.root.join(component);
        self.ensure_exact_directory(&path)?;
        Ok(path)
    }

    fn ensure_exact_directory(&self, path: &Path) -> Result<(), OwnedRootError> {
        verify_contained(&self.root, path)?;
        match fs::create_dir(path) {
            Ok(()) => self
                .created_directories
                .borrow_mut()
                .push(path.to_path_buf()),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err(OwnedRootError::new(OwnedRootErrorKind::Io)),
        }
        let metadata =
            fs::symlink_metadata(path).map_err(|_| OwnedRootError::new(OwnedRootErrorKind::Io))?;
        verify_directory_metadata(&metadata)?;
        verify_directory_no_alternate_streams(path)
    }

    fn unused_internal_path(
        &self,
        directory: &Path,
        prefix: &str,
    ) -> Result<PathBuf, OwnedRootError> {
        verify_contained(&self.root, directory)?;
        for attempt in 0..64_u64 {
            let nonce = tempfile::Builder::new()
                .prefix(prefix)
                .suffix(".name")
                .tempfile_in(directory)
                .map_err(|_| OwnedRootError::new(OwnedRootErrorKind::Io))?;
            let stem = nonce.path().file_stem().ok_or_else(containment)?.to_owned();
            drop(nonce);
            let path = directory.join(format!("{}-{attempt}.owned", stem.to_string_lossy()));
            verify_contained(&self.root, &path)?;
            if !path.exists() {
                return Ok(path);
            }
        }
        Err(OwnedRootError::new(OwnedRootErrorKind::Io))
    }
}

fn validate_root_id(root_id: &str) -> Result<(), OwnedRootError> {
    if root_id.is_empty()
        || root_id.len() > 64
        || !root_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(unverified());
    }
    Ok(())
}

fn validate_claim_token(token: &str) -> Result<(), OwnedRootError> {
    if token.is_empty()
        || token.len() > 256
        || token.contains('\0')
        || token.contains('/')
        || token.contains('\\')
        || token.contains(':')
    {
        return Err(OwnedRootError::new(OwnedRootErrorKind::InvalidClaim));
    }
    Ok(())
}

fn reject_unsafe_caller_root(path: &Path) -> Result<(), OwnedRootError> {
    if !path.is_absolute() {
        return Err(unverified());
    }
    let text = path.as_os_str().to_string_lossy();
    if text.starts_with(r"\\.\") || text.starts_with(r"\\?\") || text.contains('\0') {
        return Err(unverified());
    }
    for component in path.components() {
        match component {
            Component::Normal(value) if value.to_string_lossy().contains(':') => {
                return Err(unverified());
            }
            Component::ParentDir | Component::CurDir => return Err(unverified()),
            Component::Prefix(_) | Component::RootDir | Component::Normal(_) => {}
        }
    }
    Ok(())
}

fn verify_caller_directory_chain(path: &Path) -> Result<(), OwnedRootError> {
    let mut ancestors = path.ancestors().collect::<Vec<_>>();
    ancestors.reverse();
    for ancestor in ancestors {
        let metadata = fs::symlink_metadata(ancestor).map_err(|_| unverified())?;
        verify_directory_metadata(&metadata).map_err(|_| unverified())?;
    }
    Ok(())
}

fn verify_marker(root: &Path, expected_root_id: &str) -> Result<PhysicalIdentity, OwnedRootError> {
    let marker = root.join(OWNED_ROOT_MARKER_FILE);
    verify_contained(root, &marker)?;
    let metadata = fs::symlink_metadata(&marker).map_err(|_| unverified())?;
    verify_regular_single_link(&marker, &metadata).map_err(|_| unverified())?;
    if metadata.len() > 256 {
        return Err(unverified());
    }
    let before_identity = physical_identity_path(&marker).map_err(|_| unverified())?;
    let mut file = File::open(&marker).map_err(|_| unverified())?;
    let handle_metadata = file.metadata().map_err(|_| unverified())?;
    verify_regular_single_link(&marker, &handle_metadata).map_err(|_| unverified())?;
    if physical_identity_file(&file).map_err(|_| unverified())? != before_identity {
        return Err(unverified());
    }
    let mut bytes = Vec::with_capacity(usize::try_from(metadata.len()).unwrap_or(256));
    file.read_to_end(&mut bytes).map_err(|_| unverified())?;
    if bytes != owned_root_marker_bytes(expected_root_id) {
        return Err(unverified());
    }
    let after = fs::symlink_metadata(&marker).map_err(|_| unverified())?;
    verify_regular_single_link(&marker, &after).map_err(|_| unverified())?;
    if physical_identity_path(&marker).map_err(|_| unverified())? != before_identity {
        return Err(unverified());
    }
    Ok(before_identity)
}

fn verify_directory_chain(root: &Path, directory: &Path) -> Result<(), OwnedRootError> {
    verify_contained(root, directory)?;
    let relative = directory.strip_prefix(root).map_err(|_| containment())?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err(containment());
        };
        current.push(component);
        let metadata = fs::symlink_metadata(&current)
            .map_err(|_| OwnedRootError::new(OwnedRootErrorKind::Io))?;
        verify_directory_metadata(&metadata)?;
        verify_directory_no_alternate_streams(&current)?;
    }
    Ok(())
}

fn verify_contained(root: &Path, candidate: &Path) -> Result<(), OwnedRootError> {
    if !candidate.starts_with(root)
        || candidate
            .strip_prefix(root)
            .map_err(|_| containment())?
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(containment());
    }
    Ok(())
}

fn verify_directory_metadata(metadata: &Metadata) -> Result<(), OwnedRootError> {
    if !metadata.file_type().is_dir()
        || metadata.file_type().is_symlink()
        || is_unsafe_windows(metadata)
    {
        return Err(OwnedRootError::new(OwnedRootErrorKind::UnsafeFileKind));
    }
    Ok(())
}

fn verify_regular_single_link(path: &Path, metadata: &Metadata) -> Result<(), OwnedRootError> {
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || is_unsafe_windows(metadata)
        || link_count(path, metadata)? != 1
    {
        return Err(OwnedRootError::new(OwnedRootErrorKind::UnsafeFileKind));
    }
    verify_no_alternate_streams(path)?;
    Ok(())
}

#[cfg(windows)]
fn verify_no_alternate_streams(path: &Path) -> Result<(), OwnedRootError> {
    use std::process::Command;

    const SCRIPT: &str = "& { param($target) \
        $streams = @(Get-Item -LiteralPath $target -Stream * -ErrorAction Stop); \
        if ($streams.Count -ne 1 -or [string]$streams[0].Stream -cne ':$DATA') { exit 23 } \
    }";
    let status = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
        .arg(path)
        .status()
        .map_err(|_| OwnedRootError::new(OwnedRootErrorKind::UnsafeFileKind))?;
    if !status.success() {
        return Err(OwnedRootError::new(OwnedRootErrorKind::UnsafeFileKind));
    }
    Ok(())
}

#[cfg(windows)]
fn verify_directory_no_alternate_streams(path: &Path) -> Result<(), OwnedRootError> {
    use std::process::Command;

    const SCRIPT: &str = "& { param($target) \
        $streams = @(Get-Item -LiteralPath $target -Stream * -ErrorAction Stop); \
        if ($streams.Count -ne 0) { exit 23 } \
    }";
    let status = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", SCRIPT])
        .arg(path)
        .status()
        .map_err(|_| OwnedRootError::new(OwnedRootErrorKind::UnsafeFileKind))?;
    if !status.success() {
        return Err(OwnedRootError::new(OwnedRootErrorKind::UnsafeFileKind));
    }
    Ok(())
}

#[cfg(not(windows))]
const fn verify_no_alternate_streams(_path: &Path) -> Result<(), OwnedRootError> {
    Ok(())
}

#[cfg(not(windows))]
const fn verify_directory_no_alternate_streams(_path: &Path) -> Result<(), OwnedRootError> {
    Ok(())
}

#[cfg(windows)]
fn is_unsafe_windows(metadata: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    metadata.file_attributes() & (WINDOWS_REPARSE_POINT | WINDOWS_DEVICE) != 0
}

#[cfg(not(windows))]
const fn is_unsafe_windows(_metadata: &Metadata) -> bool {
    let _ = (WINDOWS_REPARSE_POINT, WINDOWS_DEVICE);
    false
}

#[cfg(windows)]
fn link_count(path: &Path, metadata: &Metadata) -> Result<u64, OwnedRootError> {
    use std::process::Command;
    let _ = metadata;
    let output = Command::new("fsutil.exe")
        .args(["hardlink", "list"])
        .arg(path)
        .output()
        .map_err(|_| OwnedRootError::new(OwnedRootErrorKind::UnsafeFileKind))?;
    if !output.status.success() {
        return Err(OwnedRootError::new(OwnedRootErrorKind::UnsafeFileKind));
    }
    let count = output
        .stdout
        .split(|byte| *byte == b'\n')
        .filter(|line| line.iter().any(|byte| !byte.is_ascii_whitespace()))
        .count();
    u64::try_from(count).map_err(|_| OwnedRootError::new(OwnedRootErrorKind::UnsafeFileKind))
}

#[cfg(unix)]
fn link_count(_path: &Path, metadata: &Metadata) -> Result<u64, OwnedRootError> {
    use std::os::unix::fs::MetadataExt;
    Ok(metadata.nlink())
}

fn physical_identity_path(path: &Path) -> Result<PhysicalIdentity, OwnedRootError> {
    let handle = same_file::Handle::from_path(path)
        .map_err(|_| OwnedRootError::new(OwnedRootErrorKind::UnsafeFileKind))?;
    Ok(encode_handle(&handle))
}

fn physical_identity_file(file: &File) -> Result<PhysicalIdentity, OwnedRootError> {
    let clone = file
        .try_clone()
        .map_err(|_| OwnedRootError::new(OwnedRootErrorKind::Io))?;
    let handle = same_file::Handle::from_file(clone)
        .map_err(|_| OwnedRootError::new(OwnedRootErrorKind::UnsafeFileKind))?;
    Ok(encode_handle(&handle))
}

fn encode_handle(handle: &same_file::Handle) -> PhysicalIdentity {
    let mut encoding = IdentityEncoding(Vec::new());
    handle.hash(&mut encoding);
    PhysicalIdentity(encoding.0)
}

struct IdentityEncoding(Vec<u8>);

impl Hasher for IdentityEncoding {
    fn finish(&self) -> u64 {
        0
    }

    fn write(&mut self, bytes: &[u8]) {
        self.0
            .extend_from_slice(&u64::try_from(bytes.len()).unwrap_or(u64::MAX).to_be_bytes());
        self.0.extend_from_slice(bytes);
    }
}

fn verify_bytes(
    object: &ArtifactObjectIdentity,
    declared_length: u64,
    bytes: &[u8],
) -> Result<(), OwnedRootError> {
    if u64::try_from(bytes.len()).map_err(|_| byte_limit())? != declared_length {
        return Err(OwnedRootError::new(OwnedRootErrorKind::LengthMismatch));
    }
    if digest_hex(Sha256::digest(bytes).as_slice()) != object.key().content_digest().as_str() {
        return Err(OwnedRootError::new(OwnedRootErrorKind::DigestMismatch));
    }
    Ok(())
}

fn digest_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    output
}

fn folded_path(path: &Path) -> Vec<String> {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy().to_lowercase())
        .collect()
}

fn path_prefix(left: &[String], right: &[String]) -> bool {
    left.len() <= right.len() && left.iter().zip(right).all(|(left, right)| left == right)
}

fn sync_directory(path: &Path) -> Result<(), OwnedRootError> {
    #[cfg(windows)]
    let directory = {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
        OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(path)
    };
    #[cfg(not(windows))]
    let directory = File::open(path);
    match directory.and_then(|directory| directory.sync_all()) {
        Ok(()) => Ok(()),
        #[cfg(windows)]
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::Unsupported
                    | io::ErrorKind::PermissionDenied
                    | io::ErrorKind::InvalidInput
            ) || matches!(error.raw_os_error(), Some(1 | 5)) =>
        {
            Ok(())
        }
        Err(_) => Err(OwnedRootError::new(OwnedRootErrorKind::Io)),
    }
}

const fn unverified() -> OwnedRootError {
    OwnedRootError::new(OwnedRootErrorKind::UnverifiedRoot)
}

const fn containment() -> OwnedRootError {
    OwnedRootError::new(OwnedRootErrorKind::Containment)
}

const fn byte_limit() -> OwnedRootError {
    OwnedRootError::new(OwnedRootErrorKind::ByteLimit)
}

const fn reconciliation() -> OwnedRootError {
    OwnedRootError::new(OwnedRootErrorKind::ReconciliationRequired)
}
