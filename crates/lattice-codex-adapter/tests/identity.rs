use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use lattice_codex_adapter::{
    CodexIdentityErrorKind, CodexIdentityExpectation, preflight_codex_identity,
};
use sha2::{Digest, Sha256};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy)]
enum FakeMode {
    Success,
    VersionFailure,
    SchemaFailure,
    EmptySchema,
}

struct Fixture {
    root: PathBuf,
    launcher: PathBuf,
    schema_files: Vec<(String, Vec<u8>)>,
}

impl Fixture {
    fn new(mode: FakeMode) -> Self {
        let unique = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "lattice-codex-identity-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir(&root).expect("create unique fixture root");

        let schema_files = vec![
            ("z-last.json".to_owned(), b"{\"z\":2}\n".to_vec()),
            ("v2/a-first.json".to_owned(), b"{\"a\":1}\n".to_vec()),
        ];
        let source = root.join("source");
        fs::create_dir(&source).expect("create schema source root");
        for (relative, bytes) in &schema_files {
            let path = source.join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
            fs::create_dir_all(path.parent().expect("schema source parent"))
                .expect("create schema source parent");
            fs::write(path, bytes).expect("write schema source");
        }

        let launcher = write_fake_launcher(&root, &source, mode);
        Self {
            root,
            launcher,
            schema_files,
        }
    }

    fn output(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn exact_launcher_produces_version_and_deterministic_schema_identity() {
    let fixture = Fixture::new(FakeMode::Success);
    let launcher_sha256 = sha256_bytes(&fs::read(&fixture.launcher).expect("read launcher"));
    let expectation = CodexIdentityExpectation::new(
        fixture.launcher.clone(),
        "codex-cli 0.144.6",
        launcher_sha256.clone(),
    );

    let first = preflight_codex_identity(
        &fixture.launcher,
        &expectation,
        &fixture.output("schema-one"),
    )
    .expect("first identity preflight");
    let second = preflight_codex_identity(
        &fixture.launcher,
        &expectation,
        &fixture.output("schema-two"),
    )
    .expect("second identity preflight");

    assert_eq!(first.launcher_path(), fixture.launcher);
    assert_eq!(first.version(), "codex-cli 0.144.6");
    assert_eq!(first.launcher_sha256(), launcher_sha256);
    assert_eq!(first.schema_file_count(), 2);
    assert_eq!(
        first.schema_bundle_sha256(),
        expected_schema_bundle_digest(&fixture.schema_files)
    );
    assert_eq!(first.schema_bundle_sha256(), second.schema_bundle_sha256());
}

#[test]
fn mismatched_path_version_or_digest_fails_closed() {
    let fixture = Fixture::new(FakeMode::Success);
    let digest = sha256_bytes(&fs::read(&fixture.launcher).expect("read launcher"));

    let different_path = fixture.output("different-launcher");
    fs::copy(&fixture.launcher, &different_path).expect("copy launcher");
    assert_error(
        preflight_codex_identity(
            &fixture.launcher,
            &CodexIdentityExpectation::new(different_path, "codex-cli 0.144.6", digest.clone()),
            &fixture.output("path-mismatch"),
        ),
        CodexIdentityErrorKind::LauncherPathMismatch,
    );

    assert_error(
        preflight_codex_identity(
            &fixture.launcher,
            &CodexIdentityExpectation::new(
                fixture.launcher.clone(),
                "codex-cli 0.144.5",
                digest.clone(),
            ),
            &fixture.output("version-mismatch"),
        ),
        CodexIdentityErrorKind::VersionMismatch,
    );

    assert_error(
        preflight_codex_identity(
            &fixture.launcher,
            &CodexIdentityExpectation::new(
                fixture.launcher.clone(),
                "codex-cli 0.144.6",
                "0".repeat(64),
            ),
            &fixture.output("digest-mismatch"),
        ),
        CodexIdentityErrorKind::LauncherDigestMismatch,
    );
}

#[test]
fn non_file_nonzero_and_empty_schema_results_fail_closed() {
    let non_file = Fixture::new(FakeMode::Success);
    let directory = non_file.output("launcher-directory");
    fs::create_dir(&directory).expect("create launcher directory");
    assert_error(
        preflight_codex_identity(
            &directory,
            &CodexIdentityExpectation::new(directory.clone(), "codex-cli 0.144.6", "0".repeat(64)),
            &non_file.output("non-file-schema"),
        ),
        CodexIdentityErrorKind::LauncherNotFile,
    );

    let version_failure = Fixture::new(FakeMode::VersionFailure);
    assert_error(
        preflight_with_actual_digest(&version_failure, "version-failure"),
        CodexIdentityErrorKind::VersionCommandFailed,
    );

    let schema_failure = Fixture::new(FakeMode::SchemaFailure);
    assert_error(
        preflight_with_actual_digest(&schema_failure, "schema-failure"),
        CodexIdentityErrorKind::SchemaGenerationFailed,
    );

    let empty_schema = Fixture::new(FakeMode::EmptySchema);
    assert_error(
        preflight_with_actual_digest(&empty_schema, "empty-schema"),
        CodexIdentityErrorKind::SchemaBundleEmpty,
    );
}

#[test]
fn schema_output_must_be_caller_selected_and_absent() {
    let fixture = Fixture::new(FakeMode::Success);
    let output = fixture.output("already-present");
    fs::create_dir(&output).expect("create preexisting schema directory");

    assert_error(
        preflight_with_actual_digest_at(&fixture, &output),
        CodexIdentityErrorKind::SchemaOutputExists,
    );
}

fn preflight_with_actual_digest(
    fixture: &Fixture,
    output_name: &str,
) -> Result<lattice_codex_adapter::CodexIdentityEvidence, lattice_codex_adapter::CodexIdentityError>
{
    preflight_with_actual_digest_at(fixture, &fixture.output(output_name))
}

fn preflight_with_actual_digest_at(
    fixture: &Fixture,
    output: &Path,
) -> Result<lattice_codex_adapter::CodexIdentityEvidence, lattice_codex_adapter::CodexIdentityError>
{
    let digest = sha256_bytes(&fs::read(&fixture.launcher).expect("read launcher"));
    preflight_codex_identity(
        &fixture.launcher,
        &CodexIdentityExpectation::new(fixture.launcher.clone(), "codex-cli 0.144.6", digest),
        output,
    )
}

fn assert_error<T>(
    result: Result<T, lattice_codex_adapter::CodexIdentityError>,
    expected: CodexIdentityErrorKind,
) {
    let error = result.err().expect("preflight must fail closed");
    assert_eq!(error.kind(), expected);
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex_digest(hasher.finalize().as_ref())
}

fn expected_schema_bundle_digest(files: &[(String, Vec<u8>)]) -> String {
    let mut ordered: Vec<_> = files.iter().collect();
    ordered.sort_by(|left, right| left.0.cmp(&right.0));

    let mut hasher = Sha256::new();
    hasher.update(b"lattice.codex-app-server.schema-bundle.v1\0");
    for (relative, bytes) in ordered {
        hasher.update((relative.len() as u64).to_be_bytes());
        hasher.update(relative.as_bytes());
        hasher.update((bytes.len() as u64).to_be_bytes());
        hasher.update(bytes);
    }
    hex_digest(hasher.finalize().as_ref())
}

fn hex_digest(bytes: &[u8]) -> String {
    use std::fmt::Write;

    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(&mut output, "{byte:02x}").expect("write hex digest");
    }
    output
}

#[cfg(windows)]
fn write_fake_launcher(root: &Path, source: &Path, mode: FakeMode) -> PathBuf {
    let launcher = root.join("fake-codex.cmd");
    let version_action = if matches!(mode, FakeMode::VersionFailure) {
        "exit /b 7".to_owned()
    } else {
        "echo codex-cli 0.144.6\r\nexit /b 0".to_owned()
    };
    let schema_action = match mode {
        FakeMode::SchemaFailure => "exit /b 9".to_owned(),
        FakeMode::EmptySchema => "mkdir \"%~4\"\r\nexit /b 0".to_owned(),
        FakeMode::Success | FakeMode::VersionFailure => format!(
            "mkdir \"%~4\"\r\nmkdir \"%~4\\v2\"\r\ncopy /y /b \"{}\" \"%~4\\z-last.json\" >nul\r\ncopy /y /b \"{}\" \"%~4\\v2\\a-first.json\" >nul\r\nexit /b 0",
            source.join("z-last.json").display(),
            source.join("v2").join("a-first.json").display(),
        ),
    };
    let script = format!(
        "@echo off\r\nif \"%~1\"==\"--version\" (\r\n{version_action}\r\n)\r\nif \"%~1\"==\"app-server\" if \"%~2\"==\"generate-json-schema\" if \"%~3\"==\"--out\" (\r\n{schema_action}\r\n)\r\nexit /b 11\r\n"
    );
    fs::write(&launcher, script).expect("write fake Codex launcher");
    launcher
}

#[cfg(unix)]
fn write_fake_launcher(root: &Path, source: &Path, mode: FakeMode) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;

    let launcher = root.join("fake-codex");
    let version_action = if matches!(mode, FakeMode::VersionFailure) {
        "exit 7".to_owned()
    } else {
        "printf 'codex-cli 0.144.6\\n'\nexit 0".to_owned()
    };
    let schema_action = match mode {
        FakeMode::SchemaFailure => "exit 9".to_owned(),
        FakeMode::EmptySchema => "mkdir \"$4\"\nexit 0".to_owned(),
        FakeMode::Success | FakeMode::VersionFailure => format!(
            "mkdir -p \"$4/v2\"\ncp \"{}\" \"$4/z-last.json\"\ncp \"{}\" \"$4/v2/a-first.json\"\nexit 0",
            source.join("z-last.json").display(),
            source.join("v2").join("a-first.json").display(),
        ),
    };
    let script = format!(
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then\n{version_action}\nfi\nif [ \"$1\" = \"app-server\" ] && [ \"$2\" = \"generate-json-schema\" ] && [ \"$3\" = \"--out\" ]; then\n{schema_action}\nfi\nexit 11\n"
    );
    fs::write(&launcher, script).expect("write fake Codex launcher");
    let mut permissions = fs::metadata(&launcher)
        .expect("fake launcher metadata")
        .permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&launcher, permissions).expect("make fake launcher executable");
    launcher
}
