use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::fmt::Write;
use std::fs::{self, File};
use std::io::{self, Read};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const LIVE_GATE: &str = "LATTICE_LIVE_FAULT_ACCEPTANCE";
const POSTGRES_VERSION: &str = "postgres (PostgreSQL) 17.10";
const POSTGRES_SHA256: &str = "882a5a073a88817f6c6d4c8827df1e4269ff226d52cf6f47c9883e91088c6345";
const INITDB_SHA256: &str = "2556d079888bf9ebba6b8ba7d3e8c08c947e6e564ceb73054fe1929611c87d48";
const PG_CTL_SHA256: &str = "abe89b0767a8cd0f956059aa5a5a93cd1042efc6194d000c2501da3e23babbd2";
const PG_CONTROLDATA_SHA256: &str =
    "eb48b96114795530ba9aec9920e86c41fced3bb19e4aa59781dcc4f45a31f9c3";
const POSTGRES_USER: &str = "lattice_live_owner";
const ROOT_MARKER: &str = "lattice.live-fault-root.v1";

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

    let tools = verified_postgres_tools()?;
    run_restart_infrastructure_slice(&tools)?;
    Ok(())
}

struct PostgresTools {
    initdb: PathBuf,
    pg_ctl: PathBuf,
    pg_controldata: PathBuf,
}

fn verified_postgres_tools() -> Result<PostgresTools, &'static str> {
    let postgres = verified_program("postgres.exe", POSTGRES_SHA256)?;
    let version = Command::new(&postgres)
        .arg("--version")
        .output()
        .map_err(|_| "LATTICE_LIVE_FAULT_POSTGRES_VERSION_UNAVAILABLE")?;
    if !version.status.success()
        || String::from_utf8_lossy(&version.stdout).trim() != POSTGRES_VERSION
    {
        return Err("LATTICE_LIVE_FAULT_POSTGRES_VERSION_REJECTED");
    }
    Ok(PostgresTools {
        initdb: verified_program("initdb.exe", INITDB_SHA256)?,
        pg_ctl: verified_program("pg_ctl.exe", PG_CTL_SHA256)?,
        pg_controldata: verified_program("pg_controldata.exe", PG_CONTROLDATA_SHA256)?,
    })
}

fn verified_program(file_name: &str, expected_digest: &str) -> Result<PathBuf, &'static str> {
    #[cfg(windows)]
    {
        let bin_directory = PathBuf::from(r"C:\Program Files")
            .join("PostgreSQL")
            .join("17")
            .join("bin");
        let candidate = bin_directory.join(file_name);
        if !candidate.is_file() {
            return Err("LATTICE_LIVE_FAULT_POSTGRES_IDENTITY_REJECTED");
        }
        let canonical = fs::canonicalize(&candidate)
            .map_err(|_| "LATTICE_LIVE_FAULT_POSTGRES_IDENTITY_REJECTED")?;
        let canonical_bin = fs::canonicalize(&bin_directory)
            .map_err(|_| "LATTICE_LIVE_FAULT_POSTGRES_IDENTITY_REJECTED")?;
        if canonical != canonical_bin.join(file_name)
            || sha256_file(&canonical)
                .map_err(|_| "LATTICE_LIVE_FAULT_POSTGRES_HASH_UNAVAILABLE")?
                != expected_digest
        {
            return Err("LATTICE_LIVE_FAULT_POSTGRES_IDENTITY_REJECTED");
        }
        // `initdb` resolves sibling programs itself and rejects the Windows
        // extended-path spelling returned by `canonicalize`.  Keep that form
        // only for identity verification; execute the fixed normal path.
        Ok(candidate)
    }
    #[cfg(not(windows))]
    {
        Err("LATTICE_LIVE_FAULT_WINDOWS_ONLY")
    }
}

fn run_restart_infrastructure_slice(tools: &PostgresTools) -> Result<(), &'static str> {
    let run_id = random_hex(16)?;
    let root = std::env::temp_dir().join(format!("lattice-live-fault-{run_id}"));
    fs::create_dir(&root).map_err(|_| "LATTICE_LIVE_FAULT_ROOT_CREATE_REJECTED")?;
    fs::write(root.join(ROOT_MARKER), &run_id)
        .map_err(|_| "LATTICE_LIVE_FAULT_ROOT_MARKER_REJECTED")?;
    let data = root.join("postgres-data");
    let password_path = root.join("postgres-password");
    let password = random_hex(24)?;
    fs::write(&password_path, &password).map_err(|_| "LATTICE_LIVE_FAULT_SECRET_WRITE_REJECTED")?;
    let port = reserve_loopback_port()?;

    let initdb = Command::new(&tools.initdb)
        .arg("--pgdata")
        .arg(&data)
        .arg("--username")
        .arg(POSTGRES_USER)
        .arg("--auth")
        .arg("scram-sha-256")
        .arg("--pwfile")
        .arg(&password_path)
        .output()
        .map_err(|_| "LATTICE_LIVE_FAULT_INITDB_UNAVAILABLE")?;
    if !initdb.status.success() {
        preserve_diagnostic(&root, "initdb", &initdb);
        return Err("LATTICE_LIVE_FAULT_INITDB_REJECTED");
    }

    start_cluster(&tools.pg_ctl, &data, port, &root)?;
    stop_cluster(&tools.pg_ctl, &data)?;
    let first_system_id = system_identifier(&tools.pg_controldata, &data, &root)?;
    start_cluster(&tools.pg_ctl, &data, port, &root)?;
    stop_cluster(&tools.pg_ctl, &data)?;
    let restarted_system_id = system_identifier(&tools.pg_controldata, &data, &root)?;
    if first_system_id != restarted_system_id {
        return Err("LATTICE_LIVE_FAULT_RESTART_IDENTITY_REJECTED");
    }
    // The root is intentionally preserved at this stage.  TASK-090 must add a
    // separately tested marker-and-stop-proof cleanup before deletion becomes
    // permissible; retained local evidence is safer than an unproved cleanup.
    eprintln!("LATTICE_LIVE_FAULT_INFRASTRUCTURE_RESTART_PROVED");
    Ok(())
}

fn preserve_diagnostic(root: &Path, phase: &str, output: &std::process::Output) {
    let mut bytes = output.stdout.clone();
    bytes.extend_from_slice(&output.stderr);
    let _ = fs::write(root.join(format!("{phase}.log")), bytes);
}

fn reserve_loopback_port() -> Result<u16, &'static str> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|_| "LATTICE_LIVE_FAULT_PORT_RESERVATION_REJECTED")?;
    let port = listener
        .local_addr()
        .map_err(|_| "LATTICE_LIVE_FAULT_PORT_RESERVATION_REJECTED")?
        .port();
    if matches!(port, 5432 | 55432 | 64272) {
        return Err("LATTICE_LIVE_FAULT_PORT_RESERVATION_REJECTED");
    }
    drop(listener);
    Ok(port)
}

fn start_cluster(pg_ctl: &Path, data: &Path, port: u16, root: &Path) -> Result<(), &'static str> {
    let options = format!("-p {port} -h 127.0.0.1");
    let status = Command::new(pg_ctl)
        .arg("start")
        .arg("--pgdata")
        .arg(data)
        .arg("--log")
        .arg(root.join("postgres-server.log"))
        .arg("--options")
        .arg(options)
        .arg("--wait")
        .arg("--timeout")
        .arg("30")
        .status()
        .map_err(|_| "LATTICE_LIVE_FAULT_CLUSTER_START_UNAVAILABLE")?;
    if !status.success() {
        // A failed `pg_ctl start` may still have spawned a postmaster.  This
        // data directory was freshly created and marker-owned by this run, so
        // one bounded stop attempt is safer than leaving an uncertain listener.
        // Its result is deliberately not treated as proof of cleanup.
        let _ = stop_cluster(pg_ctl, data);
        return Err("LATTICE_LIVE_FAULT_CLUSTER_START_REJECTED");
    }
    Ok(())
}

fn stop_cluster(pg_ctl: &Path, data: &Path) -> Result<(), &'static str> {
    let status = Command::new(pg_ctl)
        .arg("stop")
        .arg("--pgdata")
        .arg(data)
        .arg("--mode")
        .arg("fast")
        .arg("--wait")
        .arg("--timeout")
        .arg("30")
        .status()
        .map_err(|_| "LATTICE_LIVE_FAULT_CLUSTER_STOP_UNAVAILABLE")?;
    if !status.success() {
        return Err("LATTICE_LIVE_FAULT_CLUSTER_STOP_REJECTED");
    }
    let status = Command::new(pg_ctl)
        .arg("status")
        .arg("--pgdata")
        .arg(data)
        .output()
        .map_err(|_| "LATTICE_LIVE_FAULT_CLUSTER_STOP_UNAVAILABLE")?;
    if status.status.code() == Some(3) {
        Ok(())
    } else {
        Err("LATTICE_LIVE_FAULT_CLUSTER_STOP_UNPROVED")
    }
}

fn system_identifier(
    pg_controldata: &Path,
    data: &Path,
    root: &Path,
) -> Result<String, &'static str> {
    let output = Command::new(pg_controldata)
        .arg(data)
        .output()
        .map_err(|_| "LATTICE_LIVE_FAULT_SYSTEM_IDENTIFIER_UNAVAILABLE")?;
    if !output.status.success() {
        preserve_diagnostic(root, "system-identifier", &output);
        return Err("LATTICE_LIVE_FAULT_SYSTEM_IDENTIFIER_REJECTED");
    }
    let output_text = String::from_utf8_lossy(&output.stdout);
    let system_id = output_text
        .lines()
        .find_map(|line| {
            line.strip_prefix("Database system identifier:")
                .map(str::trim)
        })
        .unwrap_or_default()
        .to_owned();
    if system_id.is_empty() || !system_id.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("LATTICE_LIVE_FAULT_SYSTEM_IDENTIFIER_REJECTED");
    }
    Ok(system_id)
}

fn random_hex(byte_count: usize) -> Result<String, &'static str> {
    let mut bytes = vec![0_u8; byte_count];
    getrandom::fill(&mut bytes).map_err(|_| "LATTICE_LIVE_FAULT_RANDOM_UNAVAILABLE")?;
    let mut encoded = String::with_capacity(byte_count.saturating_mul(2));
    for byte in bytes {
        write!(&mut encoded, "{byte:02x}").expect("writing into String cannot fail");
    }
    Ok(encoded)
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
