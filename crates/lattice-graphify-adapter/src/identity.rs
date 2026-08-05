use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{GraphifyAdapterError, GraphifyAdapterErrorKind, GraphifyAdapterResult};
use crate::snapshot::{file_sha256, framed_digest, sha256_bytes};

/// Fixed WSL distribution containing the reviewed system execution boundary.
pub const GRAPHIFY_WSL_DISTRO: &str = "Ubuntu";
/// SHA-256 of the reviewed Windows `wsl.exe` launcher.
pub const GRAPHIFY_WSL_LAUNCHER_SHA256: &str =
    "4e589e3883229b7a74a4acdb878689dcec94e2539fcad1c194f415b149c337a9";

/// Ubuntu identity reviewed for the system-owned WSL execution boundary.
pub const GRAPHIFY_WSL_OS_ID: &str = "ubuntu";
/// Ubuntu release reviewed for the system-owned WSL execution boundary.
pub const GRAPHIFY_WSL_OS_VERSION_ID: &str = "26.04";
/// Canonical regular-file target for Ubuntu's OS identity document.
pub const GRAPHIFY_WSL_OS_RELEASE_PATH: &str = "/usr/lib/os-release";
/// SHA-256 of the reviewed Ubuntu OS identity document.
pub const GRAPHIFY_WSL_OS_RELEASE_SHA256: &str =
    "fb8b42d368e509d8b012d009dd626d0a1f860cccc105e8806da5c2ab9e59a237";

/// Fixed Python entry point inside the reviewed Ubuntu distribution.
pub const GRAPHIFY_WSL_PYTHON_PATH: &str = "/usr/bin/python3.14";
/// Exact reviewed Python version output, excluding its trailing newline.
pub const GRAPHIFY_WSL_PYTHON_VERSION: &str = "Python 3.14.4";
/// SHA-256 of the reviewed system-owned Python executable.
pub const GRAPHIFY_WSL_PYTHON_SHA256: &str =
    "fa9796cd3a30878e11a2f40372f773d3fcd913fff35e5bee8dd9a036e22e93ab";
/// SHA-256 of `python3.14 --version` with its trailing newline.
pub const GRAPHIFY_WSL_PYTHON_VERSION_SHA256: &str =
    "63990b3ee50c49ab28d70e29c0f967f0c71330e39243ffeb4d3a8d5878c8a978";

/// Fixed bubblewrap entry point inside the reviewed Ubuntu distribution.
pub const GRAPHIFY_WSL_BWRAP_PATH: &str = "/usr/bin/bwrap";
/// Exact reviewed bubblewrap version output, excluding its trailing newline.
pub const GRAPHIFY_WSL_BWRAP_VERSION: &str = "bubblewrap 0.11.1";
/// SHA-256 of the reviewed system-owned bubblewrap executable.
pub const GRAPHIFY_WSL_BWRAP_SHA256: &str =
    "8e19e40e7d5f7a7e8b488c7926feb040eab6ed10c58fa360e266d2f70670e92b";
/// SHA-256 of `bwrap --version` with its trailing newline.
pub const GRAPHIFY_WSL_BWRAP_VERSION_SHA256: &str =
    "3982b5c6efde838f903123b493520814ab1b56dfb7f35ac96af62ef443eb6acc";
/// SHA-256 of `LC_ALL=C bwrap --help` under a cleared environment.
pub const GRAPHIFY_WSL_BWRAP_HELP_SHA256: &str =
    "de21105138df92642d61bde396e49504d9d156bfea4518e9f9d8d52b93f29357";

/// Required bubblewrap capabilities whose exact use is fixed by the process adapter.
pub const GRAPHIFY_WSL_REQUIRED_BWRAP_OPTIONS: &[&str] = &[
    "--unshare-all",
    "--unshare-user",
    "--disable-userns",
    "--assert-userns-disabled",
    "--new-session",
    "--die-with-parent",
    "--cap-drop",
    "ALL",
    "--clearenv",
    "--ro-bind",
    "--bind",
    "--proc",
    "--dev",
    "--tmpfs",
    "--dir",
    "--setenv",
    "--chdir",
];

/// Canonical manifest of the complete LATTICE-owned WSL Python package payload.
pub const GRAPHIFY_WSL_RUNTIME_MANIFEST_SHA256: &str =
    "8e21411001d9f44e90ae4cf13f5e5fc1e15604bd868a75def47ad17bd31cb9d3";
/// Number of reviewed payload files, including all dependencies and the install report.
pub const GRAPHIFY_WSL_RUNTIME_FILE_COUNT: usize = 2_184;
/// Total reviewed bytes across all identity-bearing payload files.
pub const GRAPHIFY_WSL_RUNTIME_BYTE_COUNT: u64 = 159_411_927;
/// SHA-256 of the pip install report that binds package provenance and wheel hashes.
pub const GRAPHIFY_WSL_INSTALL_REPORT_SHA256: &str =
    "9901209d4cf415c16b030b8e1adeea6b216953df61115e3d9d32686ddd25a45e";
/// SHA-256 of pinned Graphify's help under the cleared WSL production environment.
pub const GRAPHIFY_WSL_GRAPHIFY_HELP_SHA256: &str =
    "8574a189c8f0621b684b2d3378b4f4e8b2f22816a497e2dfd2af38d5506c004b";
/// SHA-256 of pinned Graphify's version output under the same environment.
pub const GRAPHIFY_WSL_GRAPHIFY_VERSION_SHA256: &str =
    "0f15606f847c0ccf0790ea95c6a6dc2a3f3e654adacb9de1b790313112d988ac";
/// SHA-256 of the compile-time embedded private-copy verifier/runner.
pub const GRAPHIFY_PRIVATE_RUNNER_SHA256: &str =
    "98d0411709927a5687315f64efc6673a77f2241e2db6df8bd17c34886e3c2ad9";
/// Digest binding the reviewed system trust boundary and LATTICE-owned payload.
pub const GRAPHIFY_WSL_EXECUTION_IDENTITY_SHA256: &str =
    "f270004749c7f4fc260dfc09925b52f3b7071bcc64ba5f7cbd9bd37ae1400dd5";

const SITE_PACKAGES_RELATIVE: &str = "site-packages";
const INSTALL_REPORT_RELATIVE: &str = "install-report.json";

/// Exact production identity after both trust boundaries have been verified.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReviewedGraphifyRuntime {
    wsl_executable: PathBuf,
    runtime_root: PathBuf,
    launcher_sha256: String,
    manifest_sha256: String,
    execution_identity_sha256: String,
}

impl ReviewedGraphifyRuntime {
    #[must_use]
    pub(crate) fn wsl_executable(&self) -> &Path {
        &self.wsl_executable
    }

    #[must_use]
    pub(crate) fn runtime_root(&self) -> &Path {
        &self.runtime_root
    }

    #[must_use]
    pub(crate) fn launcher_sha256(&self) -> &str {
        &self.launcher_sha256
    }

    #[must_use]
    pub(crate) fn manifest_sha256(&self) -> &str {
        &self.manifest_sha256
    }

    #[must_use]
    pub(crate) fn execution_identity_sha256(&self) -> &str {
        &self.execution_identity_sha256
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PayloadManifest {
    entries: Vec<ManifestEntry>,
    byte_count: u64,
    manifest_sha256: String,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct ManifestEntry {
    relative_path: String,
    byte_length: u64,
    sha256: String,
}

/// Verifies the exact Windows launcher and complete LATTICE-owned WSL package payload.
///
/// Ubuntu, Python, and bubblewrap remain a system trust boundary. Their exact reviewed
/// paths, versions, binary digests, and capability-output digests are bound into the
/// returned execution identity; the process preflight must observe those constants
/// inside the fixed distribution before starting Graphify.
pub(crate) fn verify_reviewed_runtime(
    wsl_executable: &Path,
    runtime_root: &Path,
) -> GraphifyAdapterResult<ReviewedGraphifyRuntime> {
    if !wsl_executable.is_absolute() || !runtime_root.is_absolute() {
        return Err(identity_error("GRAPHIFY_WSL_RUNTIME_PATH_REJECTED"));
    }
    require_regular_file(wsl_executable, "GRAPHIFY_WSL_LAUNCHER_TYPE_REJECTED")?;
    require_regular_directory(runtime_root, "GRAPHIFY_WSL_RUNTIME_DIRECTORY_REJECTED")?;
    let wsl_executable = fs::canonicalize(wsl_executable)
        .map_err(|_| identity_error("GRAPHIFY_WSL_LAUNCHER_RESOLVE_FAILED"))?;
    let runtime_root = fs::canonicalize(runtime_root)
        .map_err(|_| identity_error("GRAPHIFY_WSL_RUNTIME_RESOLVE_FAILED"))?;
    require_regular_file(&wsl_executable, "GRAPHIFY_WSL_LAUNCHER_TYPE_REJECTED")?;
    require_regular_directory(&runtime_root, "GRAPHIFY_WSL_RUNTIME_DIRECTORY_REJECTED")?;
    if wsl_executable
        .file_name()
        .and_then(|name| name.to_str())
        .is_none_or(|name| !name.eq_ignore_ascii_case("wsl.exe"))
    {
        return Err(identity_error("GRAPHIFY_WSL_LAUNCHER_NAME_REJECTED"));
    }
    let launcher_sha256 = identity_file_sha256(&wsl_executable)?;
    if launcher_sha256 != GRAPHIFY_WSL_LAUNCHER_SHA256 {
        return Err(identity_error("GRAPHIFY_WSL_LAUNCHER_DIGEST_MISMATCH"));
    }
    verify_reviewed_wsl_system_files()?;

    let payload = collect_wsl_payload(&runtime_root)?;
    if payload.entries.len() != GRAPHIFY_WSL_RUNTIME_FILE_COUNT
        || payload.byte_count != GRAPHIFY_WSL_RUNTIME_BYTE_COUNT
    {
        return Err(identity_error("GRAPHIFY_WSL_PAYLOAD_SHAPE_MISMATCH"));
    }
    if payload.manifest_sha256 != GRAPHIFY_WSL_RUNTIME_MANIFEST_SHA256 {
        return Err(identity_error("GRAPHIFY_WSL_PAYLOAD_DIGEST_MISMATCH"));
    }
    let install_report_sha256 = identity_file_sha256(&runtime_root.join(INSTALL_REPORT_RELATIVE))?;
    if install_report_sha256 != GRAPHIFY_WSL_INSTALL_REPORT_SHA256 {
        return Err(identity_error(
            "GRAPHIFY_WSL_INSTALL_REPORT_DIGEST_MISMATCH",
        ));
    }

    let execution_identity_sha256 = reviewed_execution_identity_digest(
        &launcher_sha256,
        &payload.manifest_sha256,
        payload.entries.len(),
        payload.byte_count,
        &install_report_sha256,
    );
    if execution_identity_sha256 != GRAPHIFY_WSL_EXECUTION_IDENTITY_SHA256 {
        return Err(identity_error("GRAPHIFY_WSL_EXECUTION_IDENTITY_MISMATCH"));
    }
    Ok(ReviewedGraphifyRuntime {
        wsl_executable,
        runtime_root,
        launcher_sha256,
        manifest_sha256: payload.manifest_sha256,
        execution_identity_sha256,
    })
}

#[cfg(windows)]
fn verify_reviewed_wsl_system_files() -> GraphifyAdapterResult<()> {
    for (linux_path, expected_sha256, code) in [
        (
            GRAPHIFY_WSL_OS_RELEASE_PATH,
            GRAPHIFY_WSL_OS_RELEASE_SHA256,
            "GRAPHIFY_WSL_OS_RELEASE_DIGEST_MISMATCH",
        ),
        (
            GRAPHIFY_WSL_PYTHON_PATH,
            GRAPHIFY_WSL_PYTHON_SHA256,
            "GRAPHIFY_WSL_PYTHON_DIGEST_MISMATCH",
        ),
        (
            GRAPHIFY_WSL_BWRAP_PATH,
            GRAPHIFY_WSL_BWRAP_SHA256,
            "GRAPHIFY_WSL_BWRAP_DIGEST_MISMATCH",
        ),
    ] {
        let path = wsl_unc_path(linux_path)?;
        require_regular_file(&path, code)?;
        if identity_file_sha256(&path)? != expected_sha256 {
            return Err(identity_error(code));
        }
    }
    Ok(())
}

#[cfg(windows)]
fn wsl_unc_path(linux_path: &str) -> GraphifyAdapterResult<PathBuf> {
    let relative = linux_path
        .strip_prefix('/')
        .ok_or_else(|| identity_error("GRAPHIFY_WSL_SYSTEM_PATH_REJECTED"))?;
    if relative.is_empty()
        || relative.contains(['\\', '\0', '\r', '\n'])
        || relative.split('/').any(|component| {
            component.is_empty() || component == "." || component == ".." || !component.is_ascii()
        })
    {
        return Err(identity_error("GRAPHIFY_WSL_SYSTEM_PATH_REJECTED"));
    }
    Ok(PathBuf::from(format!(
        r"\\wsl.localhost\{}\{}",
        GRAPHIFY_WSL_DISTRO,
        relative.replace('/', r"\")
    )))
}

#[cfg(not(windows))]
fn verify_reviewed_wsl_system_files() -> GraphifyAdapterResult<()> {
    Err(identity_error("GRAPHIFY_WSL_HOST_PLATFORM_REJECTED"))
}

fn collect_wsl_payload(runtime_root: &Path) -> GraphifyAdapterResult<PayloadManifest> {
    require_regular_directory(runtime_root, "GRAPHIFY_WSL_RUNTIME_DIRECTORY_REJECTED")?;
    let site_packages = runtime_root.join(SITE_PACKAGES_RELATIVE);
    let install_report = runtime_root.join(INSTALL_REPORT_RELATIVE);
    require_regular_directory(
        &site_packages,
        "GRAPHIFY_WSL_SITE_PACKAGES_DIRECTORY_REJECTED",
    )?;
    require_regular_file(&install_report, "GRAPHIFY_WSL_INSTALL_REPORT_TYPE_REJECTED")?;

    let mut entries = Vec::with_capacity(GRAPHIFY_WSL_RUNTIME_FILE_COUNT);
    collect_payload_files(runtime_root, &site_packages, &mut entries)?;
    push_manifest_file(runtime_root, &install_report, &mut entries)?;
    entries.sort();
    if entries
        .windows(2)
        .any(|pair| pair[0].relative_path == pair[1].relative_path)
    {
        return Err(identity_error("GRAPHIFY_WSL_PAYLOAD_PATH_DUPLICATED"));
    }
    let byte_count = entries.iter().try_fold(0_u64, |total, entry| {
        total
            .checked_add(entry.byte_length)
            .ok_or_else(|| identity_error("GRAPHIFY_WSL_PAYLOAD_SIZE_OVERFLOW"))
    })?;
    let manifest_sha256 = manifest_digest(&entries)?;
    Ok(PayloadManifest {
        entries,
        byte_count,
        manifest_sha256,
    })
}

fn collect_payload_files(
    runtime_root: &Path,
    directory: &Path,
    entries: &mut Vec<ManifestEntry>,
) -> GraphifyAdapterResult<()> {
    for entry in fs::read_dir(directory)
        .map_err(|_| identity_error("GRAPHIFY_WSL_PAYLOAD_ENUMERATION_FAILED"))?
    {
        let entry = entry.map_err(|_| identity_error("GRAPHIFY_WSL_PAYLOAD_ENTRY_UNREADABLE"))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|_| identity_error("GRAPHIFY_WSL_PAYLOAD_ENTRY_TYPE_UNREADABLE"))?;
        if file_type.is_symlink() {
            return Err(identity_error("GRAPHIFY_WSL_PAYLOAD_SYMLINK_REJECTED"));
        }
        if file_type.is_dir() {
            collect_payload_files(runtime_root, &path, entries)?;
        } else if file_type.is_file() {
            if generated_bytecode_file(runtime_root, &path)? {
                continue;
            }
            push_manifest_file(runtime_root, &path, entries)?;
        } else {
            return Err(identity_error("GRAPHIFY_WSL_PAYLOAD_SPECIAL_FILE_REJECTED"));
        }
    }
    Ok(())
}

fn generated_bytecode_file(runtime_root: &Path, path: &Path) -> GraphifyAdapterResult<bool> {
    let relative = path
        .strip_prefix(runtime_root)
        .map_err(|_| identity_error("GRAPHIFY_WSL_PAYLOAD_PATH_ESCAPE"))?;
    let in_pycache = relative
        .components()
        .any(|component| component.as_os_str() == "__pycache__");
    let is_bytecode = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            extension.eq_ignore_ascii_case("pyc") || extension.eq_ignore_ascii_case("pyo")
        });
    if in_pycache && !is_bytecode {
        return Err(identity_error("GRAPHIFY_WSL_PYCACHE_NON_BYTECODE_REJECTED"));
    }
    if is_bytecode && !in_pycache {
        return Err(identity_error("GRAPHIFY_WSL_BYTECODE_LOCATION_REJECTED"));
    }
    Ok(in_pycache && is_bytecode)
}

fn push_manifest_file(
    runtime_root: &Path,
    path: &Path,
    entries: &mut Vec<ManifestEntry>,
) -> GraphifyAdapterResult<()> {
    require_regular_file(path, "GRAPHIFY_WSL_PAYLOAD_FILE_TYPE_REJECTED")?;
    let relative = path
        .strip_prefix(runtime_root)
        .map_err(|_| identity_error("GRAPHIFY_WSL_PAYLOAD_PATH_ESCAPE"))?;
    let relative = relative
        .to_str()
        .ok_or_else(|| identity_error("GRAPHIFY_WSL_PAYLOAD_PATH_NON_UTF8"))?
        .replace('\\', "/");
    if relative.is_empty()
        || !relative.is_ascii()
        || relative.contains(['\0', '\r', '\n'])
        || entries.iter().any(|entry| entry.relative_path == relative)
    {
        return Err(identity_error("GRAPHIFY_WSL_PAYLOAD_PATH_REJECTED"));
    }
    let byte_length = fs::metadata(path)
        .map_err(|_| identity_error("GRAPHIFY_WSL_PAYLOAD_METADATA_FAILED"))?
        .len();
    entries.push(ManifestEntry {
        relative_path: relative,
        byte_length,
        sha256: identity_file_sha256(path)?,
    });
    Ok(())
}

fn manifest_digest(entries: &[ManifestEntry]) -> GraphifyAdapterResult<String> {
    let mut manifest = String::new();
    for entry in entries {
        writeln!(
            manifest,
            "{}\0{}\0{}",
            entry.relative_path, entry.byte_length, entry.sha256
        )
        .map_err(|_| identity_error("GRAPHIFY_WSL_PAYLOAD_MANIFEST_BUILD_FAILED"))?;
    }
    Ok(sha256_bytes(manifest.as_bytes()))
}

fn reviewed_execution_identity_digest(
    launcher_sha256: &str,
    manifest_sha256: &str,
    file_count: usize,
    byte_count: u64,
    install_report_sha256: &str,
) -> String {
    let file_count = file_count.to_string();
    let byte_count = byte_count.to_string();
    let mut fields = vec![
        b"lattice-graphify-wsl-execution-identity-1.0".as_slice(),
        GRAPHIFY_WSL_DISTRO.as_bytes(),
        launcher_sha256.as_bytes(),
        manifest_sha256.as_bytes(),
        file_count.as_bytes(),
        byte_count.as_bytes(),
        install_report_sha256.as_bytes(),
        GRAPHIFY_WSL_GRAPHIFY_HELP_SHA256.as_bytes(),
        GRAPHIFY_WSL_GRAPHIFY_VERSION_SHA256.as_bytes(),
        GRAPHIFY_WSL_OS_ID.as_bytes(),
        GRAPHIFY_WSL_OS_VERSION_ID.as_bytes(),
        GRAPHIFY_WSL_OS_RELEASE_PATH.as_bytes(),
        GRAPHIFY_WSL_OS_RELEASE_SHA256.as_bytes(),
        GRAPHIFY_WSL_PYTHON_PATH.as_bytes(),
        GRAPHIFY_WSL_PYTHON_VERSION.as_bytes(),
        GRAPHIFY_WSL_PYTHON_SHA256.as_bytes(),
        GRAPHIFY_WSL_PYTHON_VERSION_SHA256.as_bytes(),
        GRAPHIFY_WSL_BWRAP_PATH.as_bytes(),
        GRAPHIFY_WSL_BWRAP_VERSION.as_bytes(),
        GRAPHIFY_WSL_BWRAP_SHA256.as_bytes(),
        GRAPHIFY_WSL_BWRAP_VERSION_SHA256.as_bytes(),
        GRAPHIFY_WSL_BWRAP_HELP_SHA256.as_bytes(),
        GRAPHIFY_PRIVATE_RUNNER_SHA256.as_bytes(),
        b"LATTICE_GRAPHIFY_PRIVATE_V1",
        b"runtime-input=ro-copy-verified",
        b"source-input=ro-copy-verified",
        b"runtime=private-tmpfs-landlock-write-denied",
        b"source=private-tmpfs-landlock-write-denied",
        b"output=private-tmpfs-landlock-write-allowed",
        b"landlock-abi-minimum=3",
        b"landlock-truncate-probe=runtime-install-report",
        b"capture=same-exclusive-handle",
    ];
    fields.extend(
        GRAPHIFY_WSL_REQUIRED_BWRAP_OPTIONS
            .iter()
            .map(|option| option.as_bytes()),
    );
    fields.push(b"GRAPHIFY_QUERY_LOG_DISABLE=1");
    fields.push(b"PYTHONDONTWRITEBYTECODE=1");
    fields.push(b"PYTHONPYCACHEPREFIX=/tmp/pycache");
    fields.push(b"PYTHONSAFEPATH=1");
    fields.push(b"provider-env-cleared");
    framed_digest(&fields)
}

fn identity_file_sha256(path: &Path) -> GraphifyAdapterResult<String> {
    file_sha256(path).map_err(|_| identity_error("GRAPHIFY_WSL_PAYLOAD_HASH_FAILED"))
}

fn require_regular_file(path: &Path, code: &'static str) -> GraphifyAdapterResult<()> {
    let metadata = fs::symlink_metadata(path).map_err(|_| identity_error(code))?;
    if metadata.file_type().is_file() && !metadata.file_type().is_symlink() {
        Ok(())
    } else {
        Err(identity_error(code))
    }
}

fn require_regular_directory(path: &Path, code: &'static str) -> GraphifyAdapterResult<()> {
    let metadata = fs::symlink_metadata(path).map_err(|_| identity_error(code))?;
    if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() {
        Ok(())
    } else {
        Err(identity_error(code))
    }
}

const fn identity_error(code: &'static str) -> GraphifyAdapterError {
    GraphifyAdapterError::new(GraphifyAdapterErrorKind::GraphifyIdentity, code)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    #[test]
    fn dependency_outside_graphify_package_is_part_of_wsl_payload_identity() {
        let fixture = TestDirectory::new("dependency-manifest");
        fs::create_dir_all(fixture.path().join("site-packages/graphify"))
            .expect("graphify fixture");
        fs::create_dir_all(fixture.path().join("site-packages/networkx"))
            .expect("dependency fixture");
        fs::write(
            fixture.path().join("site-packages/graphify/__init__.py"),
            b"graphify\n",
        )
        .expect("graphify bytes");
        fs::write(
            fixture.path().join("site-packages/networkx/__init__.py"),
            b"networkx\n",
        )
        .expect("dependency bytes");
        fs::write(fixture.path().join("install-report.json"), b"{}\n").expect("install report");

        let before = collect_wsl_payload(fixture.path()).expect("collect complete WSL payload");
        fs::write(
            fixture.path().join("site-packages/networkx/__init__.py"),
            b"tampered networkx\n",
        )
        .expect("tamper dependency");
        let after = collect_wsl_payload(fixture.path()).expect("recollect complete WSL payload");

        assert_ne!(before.manifest_sha256, after.manifest_sha256);
        assert!(
            before
                .entries
                .iter()
                .any(|entry| entry.relative_path == "site-packages/networkx/__init__.py")
        );
    }

    #[test]
    fn generated_bytecode_is_excluded_but_unexpected_pycache_content_is_rejected() {
        let fixture = TestDirectory::new("generated-bytecode");
        let cache = fixture.path().join("site-packages/graphify/__pycache__");
        fs::create_dir_all(&cache).expect("bytecode fixture");
        fs::write(
            fixture.path().join("site-packages/graphify/__init__.py"),
            b"graphify\n",
        )
        .expect("source bytes");
        fs::write(cache.join("__init__.cpython-314.pyc"), b"generated")
            .expect("generated bytecode");
        fs::write(fixture.path().join("install-report.json"), b"{}\n").expect("install report");

        let payload = collect_wsl_payload(fixture.path()).expect("collect without bytecode");
        assert_eq!(payload.entries.len(), 2);
        assert!(payload.entries.iter().all(|entry| {
            !Path::new(&entry.relative_path)
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("pyc"))
        }));

        fs::write(cache.join("native.so"), b"unexpected executable bytes")
            .expect("unexpected pycache content");
        assert_eq!(
            collect_wsl_payload(fixture.path())
                .expect_err("non-bytecode content under pycache must close")
                .kind(),
            GraphifyAdapterErrorKind::GraphifyIdentity
        );
    }

    #[test]
    fn execution_identity_constant_matches_all_reviewed_components() {
        assert_eq!(
            reviewed_execution_identity_digest(
                GRAPHIFY_WSL_LAUNCHER_SHA256,
                GRAPHIFY_WSL_RUNTIME_MANIFEST_SHA256,
                GRAPHIFY_WSL_RUNTIME_FILE_COUNT,
                GRAPHIFY_WSL_RUNTIME_BYTE_COUNT,
                GRAPHIFY_WSL_INSTALL_REPORT_SHA256,
            ),
            GRAPHIFY_WSL_EXECUTION_IDENTITY_SHA256
        );
    }

    #[test]
    #[ignore = "requires the reviewed Ubuntu WSL runtime paths"]
    fn live_wsl_payload_manifest_matches_reviewed_constants() {
        let runtime_root = std::env::var_os("LATTICE_TEST_GRAPHIFY_WSL_RUNTIME")
            .map(PathBuf::from)
            .expect("set reviewed WSL runtime root");
        let payload = collect_wsl_payload(&runtime_root).expect("collect reviewed WSL payload");
        println!(
            "count={} bytes={} digest={}",
            payload.entries.len(),
            payload.byte_count,
            payload.manifest_sha256
        );
        assert_eq!(payload.entries.len(), GRAPHIFY_WSL_RUNTIME_FILE_COUNT);
        assert_eq!(payload.byte_count, GRAPHIFY_WSL_RUNTIME_BYTE_COUNT);
        assert_eq!(
            payload.manifest_sha256,
            GRAPHIFY_WSL_RUNTIME_MANIFEST_SHA256
        );
    }

    #[test]
    #[ignore = "copies the reviewed WSL runtime to prove dependency tamper rejection"]
    fn reviewed_runtime_rejects_networkx_tamper_before_identity_claim() {
        let wsl_executable = std::env::var_os("LATTICE_TEST_WSL_EXE")
            .map(PathBuf::from)
            .expect("set reviewed wsl.exe");
        let source_runtime = std::env::var_os("LATTICE_TEST_GRAPHIFY_WSL_RUNTIME")
            .map(PathBuf::from)
            .expect("set reviewed WSL runtime root");
        let fixture = TestDirectory::new("live-networkx-tamper");
        copy_tree(&source_runtime, fixture.path());

        let reviewed = verify_reviewed_runtime(&wsl_executable, fixture.path())
            .expect("exact copied dependency closure remains reviewed");
        assert_eq!(
            reviewed.manifest_sha256(),
            GRAPHIFY_WSL_RUNTIME_MANIFEST_SHA256
        );
        let networkx = fixture.path().join("site-packages/networkx/__init__.py");
        let mut bytes = fs::read(&networkx).expect("read copied networkx");
        let first = bytes.first_mut().expect("networkx module is non-empty");
        *first ^= 1;
        fs::write(&networkx, bytes).expect("tamper copied networkx");

        assert_eq!(
            verify_reviewed_runtime(&wsl_executable, fixture.path())
                .expect_err("tampered dependency must not create an official identity")
                .kind(),
            GraphifyAdapterErrorKind::GraphifyIdentity
        );
    }

    fn copy_tree(source: &Path, destination: &Path) {
        fs::create_dir_all(destination).expect("create copied payload directory");
        for entry in fs::read_dir(source).expect("enumerate source payload") {
            let entry = entry.expect("read source payload entry");
            let source_path = entry.path();
            let destination_path = destination.join(entry.file_name());
            let file_type = entry.file_type().expect("read source payload type");
            if file_type.is_dir() {
                copy_tree(&source_path, &destination_path);
            } else if file_type.is_file() {
                if let Some(parent) = destination_path.parent() {
                    fs::create_dir_all(parent).expect("create copied file parent");
                }
                fs::copy(&source_path, &destination_path).expect("copy payload file");
            } else {
                panic!("unexpected source payload entry type");
            }
        }
    }

    struct TestDirectory {
        path: PathBuf,
    }

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir().join(format!(
                "lattice-graphify-wsl-identity-{label}-{}-{}",
                std::process::id(),
                TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("create identity test directory");
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
