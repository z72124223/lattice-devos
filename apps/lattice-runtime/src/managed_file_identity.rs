//! Process-local pin for executable and bridge files used by managed effects.

use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[cfg(windows)]
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
#[cfg(windows)]
use std::os::windows::io::AsRawHandle;

#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    GetFileInformationByHandle,
};

use lattice_codex_adapter::CODEX_HOME_OWNERSHIP_MARKER_NAME;
use lattice_contracts::ContentDigest;
use sha2::{Digest, Sha256};

const MAX_CONTROL_PATH_BYTES: usize = 4_096;
const MAX_CONTROL_BUNDLE_FILES: usize = 16;
const MAX_BOUNDED_EFFECT_FILES: usize = 4_096;
const MAX_CODEX_HOME_MARKER_BYTES: u64 = 1_024;
const MAX_CODEX_HOME_CONFIG_BYTES: u64 = 16 * 1_024;

/// Captures and holds deny-write/delete handles for the exact managed Codex
/// home marker and keyring-only config for the complete provider-effect
/// lifetime. This closes the validate-then-open substitution window while the
/// App Server may still open either file.
pub(crate) fn capture_managed_codex_home_guard(
    codex_home: &Path,
) -> Result<ManagedEffectBundleGuard, ()> {
    ManagedEffectBundleGuard::capture([
        (
            codex_home.join(CODEX_HOME_OWNERSHIP_MARKER_NAME),
            MAX_CODEX_HOME_MARKER_BYTES,
        ),
        (codex_home.join("config.toml"), MAX_CODEX_HOME_CONFIG_BYTES),
    ])
}

/// Returns the closed shell lookup path admitted to managed Codex children.
/// It never reads ambient `PATH`, and it rejects any admitted directory that
/// itself contains a Codex launcher so only the outer absolute
/// `LATTICE_CODEX_BIN` can start the provider.
pub(crate) fn managed_shell_path() -> Result<OsString, ()> {
    #[cfg(windows)]
    let candidates = {
        let system_root = std::env::var_os("SystemRoot")
            .or_else(|| std::env::var_os("WINDIR"))
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .ok_or(())?;
        vec![
            system_root.join("System32"),
            system_root
                .join("System32")
                .join("WindowsPowerShell")
                .join("v1.0"),
        ]
    };
    #[cfg(not(windows))]
    let candidates = vec![PathBuf::from("/usr/bin"), PathBuf::from("/bin")];

    let mut exact = Vec::new();
    for candidate in candidates {
        let canonical = fs::canonicalize(&candidate).map_err(|_| ())?;
        let metadata = fs::symlink_metadata(&canonical).map_err(|_| ())?;
        if !metadata.file_type().is_dir()
            || metadata.file_type().is_symlink()
            || is_reparse_point(&metadata)
            || ["codex", "codex.exe", "codex.cmd", "codex.ps1"]
                .iter()
                .any(|name| canonical.join(name).exists())
        {
            return Err(());
        }
        if !exact.contains(&canonical) {
            exact.push(canonical);
        }
    }
    std::env::join_paths(exact).map_err(|_| ())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PhysicalFileIdentity {
    device: u64,
    file: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AncestorPhysicalIdentity {
    path: PathBuf,
    physical: PhysicalFileIdentity,
}

/// Exact process-owned identity captured before a managed process can spawn.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ManagedFileIdentity {
    declared_path: PathBuf,
    canonical_path: PathBuf,
    length: u64,
    content_digest: ContentDigest,
    physical: PhysicalFileIdentity,
    ancestors: Vec<AncestorPhysicalIdentity>,
    identity_digest: ContentDigest,
    max_bytes: u64,
}

/// Closed set of local source files loaded by one managed process entrypoint.
///
/// Node opens ESM imports only after the OS process has started.  Callers pin
/// this complete, bounded set at adapter construction and replay it both before
/// and immediately after spawning, before sending any effect-bearing input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ManagedFileIdentityBundle {
    files: Vec<ManagedFileIdentity>,
    max_files: usize,
}

/// OS-owned handles that make a captured control bundle immutable while a
/// managed child can load or execute it.
pub(crate) struct ManagedFileSeal {
    #[cfg(windows)]
    handles: Vec<File>,
}

/// Cloneable process-lifetime guard for an already captured, closed effect
/// bundle.  The shared seal denies same-user writes, replacement, and ancestor
/// renames from service assembly until the last effect adapter/child drops it.
/// Callers still replay [`Self::verify`] immediately before every effect.
#[derive(Clone)]
pub(crate) struct ManagedEffectBundleGuard {
    identity: ManagedFileIdentityBundle,
    _lifetime_seal: Arc<ManagedFileSeal>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ManagedSealedFileSnapshot {
    canonical_path: PathBuf,
    length: u64,
    content_digest: ContentDigest,
    physical: PhysicalFileIdentity,
}

impl ManagedSealedFileSnapshot {
    pub(crate) fn canonical_path(&self) -> &Path {
        &self.canonical_path
    }

    pub(crate) const fn length(&self) -> u64 {
        self.length
    }

    pub(crate) const fn content_digest(&self) -> &ContentDigest {
        &self.content_digest
    }

    pub(crate) const fn volume_or_device(&self) -> u64 {
        self.physical.device
    }

    pub(crate) const fn file(&self) -> u64 {
        self.physical.file
    }
}

impl fmt::Debug for ManagedEffectBundleGuard {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ManagedEffectBundleGuard")
            .field("identity", &self.identity)
            .finish_non_exhaustive()
    }
}

impl PartialEq for ManagedEffectBundleGuard {
    fn eq(&self, other: &Self) -> bool {
        self.identity == other.identity
    }
}

impl Eq for ManagedEffectBundleGuard {}

impl ManagedEffectBundleGuard {
    pub(crate) fn capture<I>(files: I) -> Result<Self, ()>
    where
        I: IntoIterator<Item = (PathBuf, u64)>,
    {
        Self::from_identity(ManagedFileIdentityBundle::capture_for_seal(files)?)
    }

    /// Captures a caller-bounded effect set larger than the small bridge bundle.
    /// The hard ceiling prevents a repository inventory from turning into an
    /// unbounded handle allocation.
    pub(crate) fn capture_bounded<I>(files: I, max_files: usize) -> Result<Self, ()>
    where
        I: IntoIterator<Item = (PathBuf, u64)>,
    {
        Self::from_identity(ManagedFileIdentityBundle::capture_bounded_for_seal(
            files, max_files,
        )?)
    }

    fn from_identity(identity: ManagedFileIdentityBundle) -> Result<Self, ()> {
        let lifetime_seal = Arc::new(identity.seal()?);
        identity.verify_sealed_binding()?;
        Ok(Self {
            identity,
            _lifetime_seal: lifetime_seal,
        })
    }

    pub(crate) fn verify(&self) -> Result<(), ()> {
        // `from_identity` recomputed bytes and physical identity from every
        // held file handle before publishing the guard. Those handles deny
        // writes/deletes for this lifetime, so effect-time replay only needs to
        // prove each declared path still resolves to the held physical file and
        // exact ancestors. Re-hashing large toolchains before every Git query
        // would add unbounded CPU without strengthening the sealed invariant.
        self.identity.verify_sealed_binding()
    }

    pub(crate) fn covers_exact_file(&self, path: &Path, sha256: &str) -> Result<(), ()> {
        self.verify()?;
        let canonical = fs::canonicalize(path).map_err(|_| ())?;
        self.identity
            .files
            .iter()
            .any(|identity| {
                identity.canonical_path == canonical && identity.content_digest.as_str() == sha256
            })
            .then_some(())
            .ok_or(())
    }

    pub(crate) fn covers_file(&self, path: &Path) -> Result<(), ()> {
        self.verify()?;
        let canonical = fs::canonicalize(path).map_err(|_| ())?;
        self.identity
            .files
            .iter()
            .any(|identity| identity.canonical_path == canonical)
            .then_some(())
            .ok_or(())
    }

    pub(crate) fn sealed_file_snapshot(
        &self,
        path: &Path,
    ) -> Result<Option<ManagedSealedFileSnapshot>, ()> {
        self.verify()?;
        let canonical = fs::canonicalize(path).map_err(|_| ())?;
        Ok(self
            .identity
            .files
            .iter()
            .find(|identity| identity.canonical_path == canonical)
            .map(|identity| ManagedSealedFileSnapshot {
                canonical_path: identity.canonical_path.clone(),
                length: identity.length,
                content_digest: identity.content_digest.clone(),
                physical: identity.physical,
            }))
    }
}

impl ManagedFileIdentityBundle {
    fn capture_for_seal<I>(files: I) -> Result<Self, ()>
    where
        I: IntoIterator<Item = (PathBuf, u64)>,
    {
        Self::capture_bounded_for_seal(files, MAX_CONTROL_BUNDLE_FILES)
    }

    fn capture_bounded_for_seal<I>(files: I, max_files: usize) -> Result<Self, ()>
    where
        I: IntoIterator<Item = (PathBuf, u64)>,
    {
        if max_files == 0 || max_files > MAX_BOUNDED_EFFECT_FILES {
            return Err(());
        }
        // A process-lifetime guard immediately opens deny-write/delete handles
        // and recomputes every digest from those held handles. Avoid the two
        // path-based full-file replays used by an unsealed identity bundle: the
        // seal comparison below closes the capture-to-effect substitution lane
        // while keeping large official toolchains within the startup deadline.
        let files = files
            .into_iter()
            .map(|(path, max_bytes)| ManagedFileIdentity::capture_once(&path, max_bytes))
            .collect::<Result<Vec<_>, _>>()?;
        if files.is_empty() || files.len() > max_files {
            return Err(());
        }
        for (index, identity) in files.iter().enumerate() {
            if files[..index]
                .iter()
                .any(|prior| prior.canonical_path == identity.canonical_path)
            {
                return Err(());
            }
        }
        Ok(Self { files, max_files })
    }

    pub(crate) fn capture<I>(files: I) -> Result<Self, ()>
    where
        I: IntoIterator<Item = (PathBuf, u64)>,
    {
        Self::capture_bounded(files, MAX_CONTROL_BUNDLE_FILES)
    }

    pub(crate) fn capture_bounded<I>(files: I, max_files: usize) -> Result<Self, ()>
    where
        I: IntoIterator<Item = (PathBuf, u64)>,
    {
        if max_files == 0 || max_files > MAX_BOUNDED_EFFECT_FILES {
            return Err(());
        }
        let files = files
            .into_iter()
            .map(|(path, max_bytes)| ManagedFileIdentity::capture(&path, max_bytes))
            .collect::<Result<Vec<_>, _>>()?;
        if files.is_empty() || files.len() > max_files {
            return Err(());
        }
        for (index, identity) in files.iter().enumerate() {
            if files[..index]
                .iter()
                .any(|prior| prior.canonical_path == identity.canonical_path)
            {
                return Err(());
            }
        }
        let bundle = Self { files, max_files };
        bundle.verify()?;
        Ok(bundle)
    }

    pub(crate) fn verify(&self) -> Result<(), ()> {
        if self.files.is_empty()
            || self.files.len() > self.max_files
            || self.max_files > MAX_BOUNDED_EFFECT_FILES
        {
            return Err(());
        }
        self.files.iter().try_for_each(ManagedFileIdentity::verify)
    }

    fn verify_sealed_binding(&self) -> Result<(), ()> {
        if self.files.is_empty()
            || self.files.len() > self.max_files
            || self.max_files > MAX_BOUNDED_EFFECT_FILES
        {
            return Err(());
        }
        self.files
            .iter()
            .try_for_each(ManagedFileIdentity::verify_sealed_binding)
    }

    pub(crate) fn seal(&self) -> Result<ManagedFileSeal, ()> {
        let mut seal = ManagedFileSeal::new()?;
        for identity in &self.files {
            seal.extend(identity.seal()?);
        }
        Ok(seal)
    }
}

impl ManagedFileSeal {
    #[cfg(windows)]
    fn new() -> Result<Self, ()> {
        Ok(Self {
            handles: Vec::new(),
        })
    }

    #[cfg(not(windows))]
    fn new() -> Result<Self, ()> {
        // The product has no equivalent immutable same-user file/path seal on
        // this target. Managed effect processes therefore fail closed.
        Err(())
    }

    pub(crate) fn extend(&mut self, other: Self) {
        #[cfg(windows)]
        self.handles.extend(other.handles);
        #[cfg(not(windows))]
        let _ = other;
    }
}

impl ManagedFileIdentity {
    pub(crate) fn capture(path: &Path, max_bytes: u64) -> Result<Self, ()> {
        let identity = Self::capture_once(path, max_bytes)?;
        identity.verify()?;
        Ok(identity)
    }

    fn capture_once(path: &Path, max_bytes: u64) -> Result<Self, ()> {
        if !path.is_absolute() || max_bytes == 0 {
            return Err(());
        }
        validate_path_chain(path)?;
        let declared_path = path.to_path_buf();
        let canonical_path = fs::canonicalize(path).map_err(|_| ())?;
        validate_path_chain(&canonical_path)?;
        let canonical_text = canonical_path.to_str().ok_or(())?;
        if canonical_text.is_empty() || canonical_text.len() > MAX_CONTROL_PATH_BYTES {
            return Err(());
        }
        let (length, content_digest, physical) = capture_open_file(&canonical_path, max_bytes)?;
        let ancestors = capture_ancestor_identities(&canonical_path)?;
        let identity_digest = identity_digest(
            canonical_text,
            length,
            &content_digest,
            physical,
            &ancestors,
        )?;
        Ok(Self {
            declared_path,
            canonical_path,
            length,
            content_digest,
            physical,
            ancestors,
            identity_digest,
            max_bytes,
        })
    }

    pub(crate) fn verify(&self) -> Result<(), ()> {
        validate_path_chain(&self.declared_path)?;
        let canonical_path = fs::canonicalize(&self.declared_path).map_err(|_| ())?;
        if canonical_path != self.canonical_path {
            return Err(());
        }
        validate_path_chain(&canonical_path)?;
        let (length, content_digest, physical) =
            capture_open_file(&canonical_path, self.max_bytes)?;
        let ancestors = capture_ancestor_identities(&canonical_path)?;
        if length != self.length
            || content_digest != self.content_digest
            || physical != self.physical
            || ancestors != self.ancestors
        {
            return Err(());
        }
        let observed = identity_digest(
            canonical_path.to_str().ok_or(())?,
            length,
            &content_digest,
            physical,
            &ancestors,
        )?;
        (observed == self.identity_digest).then_some(()).ok_or(())
    }

    pub(crate) fn seal(&self) -> Result<ManagedFileSeal, ()> {
        seal_file_identity(self)
    }

    fn verify_sealed_binding(&self) -> Result<(), ()> {
        validate_path_chain(&self.declared_path)?;
        let canonical_path = fs::canonicalize(&self.declared_path).map_err(|_| ())?;
        if canonical_path != self.canonical_path {
            return Err(());
        }
        validate_path_chain(&canonical_path)?;
        let file = File::open(&canonical_path).map_err(|_| ())?;
        let metadata = file.metadata().map_err(|_| ())?;
        if !metadata.file_type().is_file()
            || is_reparse_point(&metadata)
            || metadata.len() != self.length
            || physical_identity(&file)? != self.physical
            || capture_ancestor_identities(&canonical_path)? != self.ancestors
        {
            return Err(());
        }
        Ok(())
    }
}

#[cfg(windows)]
fn seal_file_identity(identity: &ManagedFileIdentity) -> Result<ManagedFileSeal, ()> {
    seal_file_identity_with_pre_open_hook(identity, || {})
}

#[cfg(windows)]
fn seal_file_identity_with_pre_open_hook(
    identity: &ManagedFileIdentity,
    pre_file_open_hook: impl FnOnce(),
) -> Result<ManagedFileSeal, ()> {
    let mut handles = Vec::with_capacity(identity.ancestors.len().saturating_add(1));
    for ancestor in &identity.ancestors {
        let directory = open_sealed_path(&ancestor.path, true)?;
        let metadata = directory.metadata().map_err(|_| ())?;
        if !metadata.file_type().is_dir()
            || is_reparse_point(&metadata)
            || physical_identity(&directory)? != ancestor.physical
        {
            return Err(());
        }
        handles.push(directory);
    }
    pre_file_open_hook();
    let file = open_sealed_path(&identity.canonical_path, false)?;
    let (length, content_digest, physical) = capture_held_file(&file, identity.max_bytes)?;
    if length != identity.length
        || content_digest != identity.content_digest
        || physical != identity.physical
    {
        return Err(());
    }
    handles.push(file);
    // Directory handles deny rename/delete of every declared path component;
    // the file handle additionally denies writes and replacement. Replay only
    // after all handles are held, closing the final admission race.
    // Rebind the caller-supplied spelling to the captured canonical path only
    // after exact ancestor and file handles are held. Content and physical ID
    // were recomputed from the held file itself; repeating a path-based hash
    // would add cost without strengthening the already deny-write/delete seal.
    validate_path_chain(&identity.declared_path)?;
    if fs::canonicalize(&identity.declared_path).map_err(|_| ())? != identity.canonical_path {
        return Err(());
    }
    validate_path_chain(&identity.canonical_path)?;
    Ok(ManagedFileSeal { handles })
}

#[cfg(not(windows))]
fn seal_file_identity(_identity: &ManagedFileIdentity) -> Result<ManagedFileSeal, ()> {
    Err(())
}

#[cfg(windows)]
fn open_sealed_path(path: &Path, directory: bool) -> Result<File, ()> {
    let share = if directory {
        FILE_SHARE_READ | FILE_SHARE_WRITE
    } else {
        FILE_SHARE_READ
    };
    let flags = if directory {
        FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT
    } else {
        FILE_FLAG_OPEN_REPARSE_POINT
    };
    OpenOptions::new()
        .read(true)
        .share_mode(share)
        .custom_flags(flags)
        .open(path)
        .map_err(|_| ())
}

fn capture_open_file(
    path: &Path,
    max_bytes: u64,
) -> Result<(u64, ContentDigest, PhysicalFileIdentity), ()> {
    let metadata = fs::symlink_metadata(path).map_err(|_| ())?;
    if !metadata.file_type().is_file()
        || metadata.file_type().is_symlink()
        || is_reparse_point(&metadata)
        || metadata.len() > max_bytes
    {
        return Err(());
    }
    let file = File::open(path).map_err(|_| ())?;
    let (length, content_digest, physical) = capture_held_file(&file, max_bytes)?;
    if length != metadata.len() {
        return Err(());
    }
    Ok((length, content_digest, physical))
}

fn capture_held_file(
    file: &File,
    max_bytes: u64,
) -> Result<(u64, ContentDigest, PhysicalFileIdentity), ()> {
    let opened = file.metadata().map_err(|_| ())?;
    if !opened.file_type().is_file() || is_reparse_point(&opened) || opened.len() > max_bytes {
        return Err(());
    }
    let physical = physical_identity(file)?;
    let mut reader = file.try_clone().map_err(|_| ())?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1_024];
    let mut total = 0_u64;
    loop {
        let read = reader.read(&mut buffer).map_err(|_| ())?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(u64::try_from(read).map_err(|_| ())?)
            .ok_or(())?;
        if total > max_bytes || total > opened.len() {
            return Err(());
        }
        hasher.update(&buffer[..read]);
    }
    if total != opened.len() || physical_identity(file)? != physical {
        return Err(());
    }
    let content_digest = digest_from_hasher(hasher)?;
    Ok((total, content_digest, physical))
}

fn capture_ancestor_identities(path: &Path) -> Result<Vec<AncestorPhysicalIdentity>, ()> {
    let mut paths = path.parent().ok_or(())?.ancestors().collect::<Vec<_>>();
    paths.reverse();
    if paths.is_empty() || paths.len() > 128 {
        return Err(());
    }
    paths
        .into_iter()
        .map(|path| {
            let path = path.to_path_buf();
            if path
                .to_str()
                .is_none_or(|text| text.is_empty() || text.len() > MAX_CONTROL_PATH_BYTES)
            {
                return Err(());
            }
            let directory = open_directory_for_identity(&path)?;
            let metadata = directory.metadata().map_err(|_| ())?;
            if !metadata.file_type().is_dir() || is_reparse_point(&metadata) {
                return Err(());
            }
            Ok(AncestorPhysicalIdentity {
                path,
                physical: physical_identity(&directory)?,
            })
        })
        .collect()
}

#[cfg(windows)]
fn open_directory_for_identity(path: &Path) -> Result<File, ()> {
    OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|_| ())
}

#[cfg(not(windows))]
fn open_directory_for_identity(path: &Path) -> Result<File, ()> {
    File::open(path).map_err(|_| ())
}

fn validate_path_chain(path: &Path) -> Result<(), ()> {
    let mut current = Some(path);
    while let Some(candidate) = current {
        let metadata = fs::symlink_metadata(candidate).map_err(|_| ())?;
        if metadata.file_type().is_symlink() || is_reparse_point(&metadata) {
            return Err(());
        }
        current = candidate.parent();
    }
    Ok(())
}

#[cfg(windows)]
fn is_reparse_point(metadata: &Metadata) -> bool {
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse_point(_metadata: &Metadata) -> bool {
    false
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn physical_identity(file: &File) -> Result<PhysicalFileIdentity, ()> {
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    if unsafe { GetFileInformationByHandle(file.as_raw_handle().cast(), &raw mut information) } == 0
    {
        return Err(());
    }
    Ok(PhysicalFileIdentity {
        device: u64::from(information.dwVolumeSerialNumber),
        file: (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow),
    })
}

#[cfg(unix)]
fn physical_identity(file: &File) -> Result<PhysicalFileIdentity, ()> {
    use std::os::unix::fs::MetadataExt;
    let metadata = file.metadata().map_err(|_| ())?;
    Ok(PhysicalFileIdentity {
        device: metadata.dev(),
        file: metadata.ino(),
    })
}

#[cfg(not(any(windows, unix)))]
fn physical_identity(_file: &File) -> Result<PhysicalFileIdentity, ()> {
    Err(())
}

fn identity_digest(
    canonical_path: &str,
    length: u64,
    content_digest: &ContentDigest,
    physical: PhysicalFileIdentity,
    ancestors: &[AncestorPhysicalIdentity],
) -> Result<ContentDigest, ()> {
    let mut hasher = Sha256::new();
    for bytes in [
        canonical_path.as_bytes(),
        &length.to_be_bytes(),
        content_digest.as_str().as_bytes(),
        &physical.device.to_be_bytes(),
        &physical.file.to_be_bytes(),
    ] {
        hasher.update(u64::try_from(bytes.len()).map_err(|_| ())?.to_be_bytes());
        hasher.update(bytes);
    }
    hasher.update(
        u64::try_from(ancestors.len())
            .map_err(|_| ())?
            .to_be_bytes(),
    );
    for ancestor in ancestors {
        let path = ancestor.path.to_str().ok_or(())?.as_bytes();
        for bytes in [
            path,
            &ancestor.physical.device.to_be_bytes(),
            &ancestor.physical.file.to_be_bytes(),
        ] {
            hasher.update(u64::try_from(bytes.len()).map_err(|_| ())?.to_be_bytes());
            hasher.update(bytes);
        }
    }
    digest_from_hasher(hasher)
}

fn digest_from_hasher(hasher: Sha256) -> Result<ContentDigest, ()> {
    let bytes = hasher.finalize();
    let mut encoded = String::with_capacity(64);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut encoded, "{byte:02x}").map_err(|_| ())?;
    }
    ContentDigest::from_sha256(encoded).map_err(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lattice_codex_adapter::{
        CODEX_HOME_CONFIG_BYTES, CODEX_HOME_OWNERSHIP_MARKER_BYTES,
        CODEX_HOME_OWNERSHIP_MARKER_NAME,
    };
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQUENCE: AtomicU64 = AtomicU64::new(1);

    fn fixture() -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "lattice-managed-file-identity-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).expect("identity fixture root");
        let path = root.join("runner.bin");
        fs::write(&path, b"trusted-runner-v1\n").expect("identity fixture file");
        (root, path)
    }

    #[test]
    fn same_bytes_at_different_paths_have_different_identity() {
        let (root, first) = fixture();
        let second = root.join("other.bin");
        fs::write(&second, b"trusted-runner-v1\n").expect("second file");
        let first = ManagedFileIdentity::capture(&first, 1024).expect("first identity");
        let second = ManagedFileIdentity::capture(&second, 1024).expect("second identity");
        assert_eq!(first.content_digest, second.content_digest);
        assert_ne!(first.identity_digest, second.identity_digest);
        fs::remove_dir_all(root).expect("remove identity fixture");
    }

    #[test]
    fn replacement_and_oversize_are_rejected_before_effect() {
        let (root, path) = fixture();
        let identity = ManagedFileIdentity::capture(&path, 1024).expect("captured identity");
        fs::write(&path, b"replacement-runner\n").expect("replace runner");
        assert!(identity.verify().is_err());
        fs::write(&path, vec![b'x'; 1025]).expect("oversized runner");
        assert!(ManagedFileIdentity::capture(&path, 1024).is_err());
        fs::remove_dir_all(root).expect("remove identity fixture");
    }

    #[test]
    fn bundle_replay_rejects_transitive_substitution() {
        let (root, entry) = fixture();
        let dependency = root.join("dependency.mjs");
        fs::write(&dependency, b"export const trusted = true;\n").expect("dependency");
        let bundle =
            ManagedFileIdentityBundle::capture([(entry, 1024), (dependency.clone(), 1024)])
                .expect("closed bundle");
        fs::write(&dependency, b"export const trusted = false;\n").expect("substitute dependency");
        assert!(bundle.verify().is_err());
        fs::remove_dir_all(root).expect("remove identity fixture");
    }

    #[cfg(windows)]
    #[test]
    fn bundle_seal_denies_same_user_write_and_delete_until_drop() {
        let (root, entry) = fixture();
        let dependency = root.join("dependency.mjs");
        fs::write(&dependency, b"export const trusted = true;\n").expect("dependency");
        let bundle =
            ManagedFileIdentityBundle::capture([(entry, 1024), (dependency.clone(), 1024)])
                .expect("closed bundle");
        let seal = bundle.seal().expect("immutable OS bundle seal");
        assert!(
            fs::write(&dependency, b"export const trusted = false;\n").is_err(),
            "deny-write handle must close the preverify/spawn ABA lane"
        );
        assert!(
            fs::remove_file(&dependency).is_err(),
            "deny-delete file and ancestor handles must reject replacement"
        );
        bundle.verify().expect("sealed bundle remains exact");
        drop(seal);
        fs::write(&dependency, b"export const trusted = false;\n")
            .expect("write permitted only after seal release");
        fs::remove_dir_all(root).expect("remove identity fixture");
    }

    #[cfg(windows)]
    #[test]
    fn cloned_effect_guard_retains_denial_until_last_service_or_child_owner_drops() {
        let (root, entry) = fixture();
        let dependency = root.join("official-resource.bin");
        fs::write(&dependency, b"trusted-official-resource\n").expect("resource");
        let expected = ManagedFileIdentity::capture(&dependency, 1024)
            .expect("resource identity")
            .content_digest;
        let service_guard =
            ManagedEffectBundleGuard::capture([(entry, 1024), (dependency.clone(), 1024)])
                .expect("process-lifetime guard");
        service_guard
            .covers_exact_file(&dependency, expected.as_str())
            .expect("exact sealed file is covered");
        assert!(
            service_guard
                .covers_exact_file(
                    &dependency,
                    "0000000000000000000000000000000000000000000000000000000000000000"
                )
                .is_err(),
            "a different digest must not borrow the external seal"
        );
        let child_guard = service_guard.clone();
        drop(service_guard);
        assert!(
            fs::write(&dependency, b"replacement\n").is_err(),
            "child clone must retain the assembly-time denial"
        );
        child_guard.verify().expect("held bundle remains current");
        drop(child_guard);
        fs::write(&dependency, b"replacement\n").expect("released after last owner");
        fs::remove_dir_all(root).expect("remove identity fixture");
    }

    #[cfg(windows)]
    #[test]
    fn managed_codex_home_guard_seals_marker_and_keyring_config_for_process_lifetime() {
        let root = std::env::temp_dir().join(format!(
            "lattice-managed-codex-home-seal-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).expect("Codex home fixture root");
        let marker = root.join(CODEX_HOME_OWNERSHIP_MARKER_NAME);
        let config = root.join("config.toml");
        fs::write(&marker, CODEX_HOME_OWNERSHIP_MARKER_BYTES).expect("owned-home marker");
        fs::write(&config, CODEX_HOME_CONFIG_BYTES).expect("keyring-only config");

        let guard = capture_managed_codex_home_guard(&root).expect("sealed managed Codex home");
        assert!(fs::write(&marker, b"substituted\n").is_err());
        assert!(fs::remove_file(&config).is_err());
        guard.verify().expect("sealed home stays exact");
        drop(guard);

        fs::write(&config, CODEX_HOME_CONFIG_BYTES).expect("write allowed after effect lifetime");
        fs::remove_dir_all(root).expect("remove home fixture");
    }

    #[cfg(windows)]
    #[test]
    fn pre_open_aba_substitution_cannot_seal_the_wrong_physical_file() {
        let (root, path) = fixture();
        let retained = root.join("retained-original.bin");
        let malicious = root.join("malicious.bin");
        fs::write(&malicious, b"malicious-import-time-effect\n").expect("malicious file");
        let identity = ManagedFileIdentity::capture(&path, 1024).expect("captured original");
        let result = seal_file_identity_with_pre_open_hook(&identity, || {
            fs::rename(&path, &retained).expect("move original during failpoint");
            fs::rename(&malicious, &path).expect("substitute malicious path during failpoint");
        });
        assert!(
            result.is_err(),
            "opened handle identity must reject ABA substitute"
        );
        fs::remove_file(&path).expect("remove malicious substitute");
        fs::rename(&retained, &path).expect("restore original");
        identity
            .verify()
            .expect("original identity restored for cleanup");
        fs::remove_dir_all(root).expect("remove identity fixture");
    }
}
