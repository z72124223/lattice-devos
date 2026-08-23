use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::fmt::Write;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const LIVE_GATE: &str = "LATTICE_LIVE_FAULT_ACCEPTANCE";
const POSTGRES_VERSION: &str = "PostgreSQL 17.10";
const POSTGRES_SHA256: &str = "882a5a073a88817f6c6d4c8827df1e4269ff226d52cf6f47c9883e91088c6345";

fn main() -> ExitCode {
    match run(
        std::env::args_os().skip(1).collect(),
        std::env::var(LIVE_GATE).ok(),
    ) {
        Ok(()) => ExitCode::SUCCESS,
        Err(code) => {
            eprintln!("{code}");
            ExitCode::from(2)
        }
    }
}

fn run(arguments: Vec<OsString>, live_gate: Option<String>) -> Result<(), &'static str> {
    if !arguments.is_empty() {
        return Err("LATTICE_LIVE_FAULT_ARGUMENTS_REJECTED");
    }
    if live_gate.as_deref() != Some("1") {
        return Err("LATTICE_LIVE_FAULT_OPT_IN_REQUIRED");
    }

    let postgres = expected_postgres_path()?;
    let canonical = std::fs::canonicalize(&postgres)
        .map_err(|_| "LATTICE_LIVE_FAULT_POSTGRES_IDENTITY_REJECTED")?;
    if canonical != postgres {
        return Err("LATTICE_LIVE_FAULT_POSTGRES_IDENTITY_REJECTED");
    }
    let version = Command::new(&canonical)
        .arg("--version")
        .output()
        .map_err(|_| "LATTICE_LIVE_FAULT_POSTGRES_VERSION_UNAVAILABLE")?;
    if !version.status.success()
        || String::from_utf8_lossy(&version.stdout).trim() != POSTGRES_VERSION
    {
        return Err("LATTICE_LIVE_FAULT_POSTGRES_VERSION_REJECTED");
    }
    let digest =
        sha256_file(&canonical).map_err(|_| "LATTICE_LIVE_FAULT_POSTGRES_HASH_UNAVAILABLE")?;
    if digest != POSTGRES_SHA256 {
        return Err("LATTICE_LIVE_FAULT_POSTGRES_IDENTITY_REJECTED");
    }

    // This binary is deliberately only the safety foundation.  It has not yet
    // created a marker-owned cluster, so a readiness result cannot be mistaken
    // for live fault-recovery acceptance.
    eprintln!("LATTICE_LIVE_FAULT_FOUNDATION_READY");
    Err("LATTICE_LIVE_FAULT_CLUSTER_RUNNER_NOT_IMPLEMENTED")
}

fn expected_postgres_path() -> Result<PathBuf, &'static str> {
    #[cfg(windows)]
    {
        let program_files = std::env::var_os("ProgramFiles")
            .ok_or("LATTICE_LIVE_FAULT_POSTGRES_IDENTITY_REJECTED")?;
        let candidate = PathBuf::from(program_files)
            .join("PostgreSQL")
            .join("17")
            .join("bin")
            .join("postgres.exe");
        if !candidate.is_file() {
            return Err("LATTICE_LIVE_FAULT_POSTGRES_IDENTITY_REJECTED");
        }
        Ok(candidate)
    }
    #[cfg(not(windows))]
    {
        Err("LATTICE_LIVE_FAULT_WINDOWS_ONLY")
    }
}

fn sha256_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 8_192];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    let mut encoded = String::with_capacity(64);
    for byte in hasher.finalize() {
        write!(&mut encoded, "{byte:02x}").expect("writing into String cannot fail");
    }
    Ok(encoded)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_arguments_before_any_live_preflight() {
        assert_eq!(
            run(vec![OsString::from("unexpected")], Some("1".to_owned())),
            Err("LATTICE_LIVE_FAULT_ARGUMENTS_REJECTED")
        );
    }

    #[test]
    fn requires_an_explicit_live_opt_in() {
        assert_eq!(
            run(Vec::new(), None),
            Err("LATTICE_LIVE_FAULT_OPT_IN_REQUIRED")
        );
        assert_eq!(
            run(Vec::new(), Some("yes".to_owned())),
            Err("LATTICE_LIVE_FAULT_OPT_IN_REQUIRED")
        );
    }
}
