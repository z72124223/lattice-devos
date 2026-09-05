//! Trusted maintenance CLI: execute one fixed Node verifier, retain its bytes,
//! then let Runtime append the typed result to the existing Task Ledger.
use crate::external_result_import::{
    digest, digest_bytes, file_bytes, file_digest, hex, json_file, run_git, text,
};
use lattice_artifact_store::LocalVerifiedResultEvidence;
use lattice_task_ledger::{LocalVerifiedResultAdoption, TaskSubmissionEnvelope};
use postgres::{Client, IsolationLevel};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

type Result<T> = std::result::Result<T, &'static str>;
const REJECTED: &str = "LATTICE_LOCAL_RESULT_IMPORT_REJECTED";
const MISMATCH: &str = "LATTICE_LOCAL_RESULT_EVIDENCE_MISMATCH";
const DATABASE: &str = "LATTICE_LOCAL_RESULT_DATABASE_UNAVAILABLE";

pub(crate) struct ImportRequest {
    root: PathBuf,
    workspace: PathBuf,
    pub(crate) adoption: LocalVerifiedResultAdoption,
}

pub(crate) fn parse(path: &Path) -> Result<ImportRequest> {
    if !path.is_absolute() {
        return Err(REJECTED);
    }
    let value = json_file(path)?;
    let fields = [
        "schema",
        "evidence_root",
        "workspace",
        "task_ref",
        "client_request_id",
        "expected_ledger_head_digest",
        "artifact_ref",
        "acceptance_ref",
    ];
    let object = value.as_object().ok_or(REJECTED)?;
    if object.len() != fields.len()
        || fields.iter().any(|key| !object.contains_key(*key))
        || text(&value, "schema")? != "lattice.local-result-import.v1"
    {
        return Err(REJECTED);
    }
    let canonical = |field| -> Result<PathBuf> {
        let path = Path::new(text(&value, field)?);
        if !path.is_absolute() {
            return Err(REJECTED);
        }
        let path = fs::canonicalize(path).map_err(|_| REJECTED)?;
        if !path.is_dir() {
            return Err(REJECTED);
        }
        Ok(path)
    };
    Ok(ImportRequest {
        root: canonical("evidence_root")?,
        workspace: canonical("workspace")?,
        adoption: LocalVerifiedResultAdoption::new(
            digest(text(&value, "task_ref")?)?,
            text(&value, "client_request_id")?,
            digest(text(&value, "expected_ledger_head_digest")?)?,
            text(&value, "artifact_ref")?,
            text(&value, "acceptance_ref")?,
        )
        .map_err(|_| REJECTED)?,
    })
}

impl ImportRequest {
    fn receipt(&self, reference: &str, kind: &str) -> Result<Value> {
        let expected = digest(reference.strip_prefix("evidence:sha256:").ok_or(REJECTED)?)?;
        let path = fs::canonicalize(self.root.join(format!("{}.json", expected.as_str())))
            .map_err(|_| REJECTED)?;
        if !path.starts_with(&self.root) {
            return Err(REJECTED);
        }
        let bytes = file_bytes(&path, 1024 * 1024)?;
        if hex(&Sha256::digest(&bytes)) != expected.as_str() {
            return Err(MISMATCH);
        }
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| REJECTED)?;
        if text(&value, "schema")? != format!("lattice.local-result.{kind}.v1")
            || text(&value, "task_ref")? != self.adoption.task_ref().as_str()
        {
            return Err(MISMATCH);
        }
        Ok(value)
    }
    fn scoped_file(&self, path: &str) -> Result<PathBuf> {
        let path = Path::new(path);
        if !path.is_absolute() {
            return Err(REJECTED);
        }
        let path = fs::canonicalize(path).map_err(|_| REJECTED)?;
        if !path.starts_with(&self.workspace) || !path.is_file() {
            return Err(REJECTED);
        }
        Ok(path)
    }
    pub(crate) fn verify_and_retain(
        &self,
        submission: &TaskSubmissionEnvelope,
        repository: &Path,
        git: &Path,
        node: &Path,
        migrator: &mut Client,
    ) -> Result<Value> {
        if submission.task_ref() != self.adoption.task_ref()
            || submission.client_request_id() != self.adoption.client_request_id()
        {
            return Err(MISMATCH);
        }
        let repository = fs::canonicalize(repository).map_err(|_| REJECTED)?;
        if self.workspace != repository {
            let args = ["rev-parse", "--path-format=absolute", "--git-common-dir"];
            let expected = run_git(git, &repository, &args)?;
            let observed = run_git(git, &self.workspace, &args)?;
            if fs::canonicalize(expected.trim()).map_err(|_| REJECTED)?
                != fs::canonicalize(observed.trim()).map_err(|_| REJECTED)?
            {
                return Err(MISMATCH);
            }
        }
        let artifact = self.receipt(self.adoption.artifact_ref(), "artifact")?;
        let acceptance = self.receipt(self.adoption.acceptance_ref(), "acceptance")?;
        let artifact_path = self.scoped_file(text(&artifact, "path")?)?;
        let artifact_hash = file_digest(&artifact_path)?;
        let test_path = self.scoped_file(text(&acceptance, "test_path")?)?;
        let test_hash = file_digest(&test_path)?;
        if artifact_hash != digest(text(&artifact, "sha256")?)?
            || test_hash != digest(text(&acceptance, "test_sha256")?)?
            || text(&acceptance, "artifact_ref")? != self.adoption.artifact_ref()
            || text(&acceptance, "runner_profile")? != LocalVerifiedResultEvidence::RUNNER_PROFILE
            || text(&acceptance, "executor_id")? == text(&acceptance, "verifier_id")?
        {
            return Err(MISMATCH);
        }
        let node_hash = file_digest(node)?;
        let (passed, output) = run_node(node, &self.workspace, &test_path)?;
        if file_digest(&artifact_path)? != artifact_hash
            || file_digest(&test_path)? != test_hash
            || file_digest(node)? != node_hash
        {
            return Err(MISMATCH);
        }
        let output_hash = digest(&hex(&Sha256::digest(&output)))?;
        let output_path = self.root.join(format!("{}.log", output_hash.as_str()));
        retain_bytes(&output_path, &output)?;
        if !passed {
            return Err("LATTICE_LOCAL_RESULT_VERIFICATION_FAILED");
        }

        let mut tx = migrator
            .build_transaction()
            .isolation_level(IsolationLevel::Serializable)
            .start()
            .map_err(|_| DATABASE)?;
        tx.batch_execute("SET LOCAL lock_timeout='5s'; SET LOCAL statement_timeout='30s'")
            .map_err(|_| DATABASE)?;
        let role: bool = tx.query_one("SELECT session_user='lattice_migrator_login' AND current_setting('role')='lattice_migrator'", &[]).map_err(|_| DATABASE)?.get(0);
        if !role {
            return Err(REJECTED);
        }
        let adoption_hash = digest_bytes(self.adoption.result_digest());
        let existing = tx.query_opt("SELECT acceptance_sha256 FROM ONLY control_product.local_verified_result_evidence WHERE adoption_digest=$1 FOR SHARE", &[&adoption_hash]).map_err(|_| DATABASE)?;
        let retained_output = if let Some(row) = existing {
            let hash = digest(&hex(&row.get::<_, Vec<u8>>(0)))?;
            if file_digest(&self.root.join(format!("{}.log", hash.as_str())))? != hash {
                return Err(MISMATCH);
            }
            hash
        } else {
            output_hash
        };
        let evidence = LocalVerifiedResultEvidence::new(
            submission.identity().project_id().clone(),
            submission.identity().project_snapshot_id().clone(),
            &self.adoption,
            artifact_hash,
            retained_output,
            text(&acceptance, "verifier_id")?,
            LocalVerifiedResultEvidence::RUNNER_PROFILE,
        )?;
        let artifact_hash = digest_bytes(evidence.artifact_sha256());
        let acceptance_hash = digest_bytes(evidence.acceptance_sha256());
        let descriptor = digest_bytes(evidence.descriptor_digest());
        let expected_head = digest_bytes(self.adoption.expected_ledger_head_digest());
        let params: &[&(dyn postgres::types::ToSql + Sync)] = &[
            &adoption_hash,
            &evidence.project_id().as_str(),
            &evidence.project_snapshot_id().as_str(),
            &self.adoption.task_ref().as_str(),
            &self.adoption.client_request_id(),
            &expected_head,
            &self.adoption.artifact_ref(),
            &self.adoption.acceptance_ref(),
            &artifact_hash,
            &acceptance_hash,
            &evidence.independent_verifier(),
            &LocalVerifiedResultEvidence::RUNNER_PROFILE,
            &descriptor,
        ];
        let inserted = tx.execute("INSERT INTO control_product.local_verified_result_evidence(adoption_digest,project_id,project_snapshot_id,task_ref,client_request_id,expected_head_digest,artifact_ref,acceptance_ref,artifact_sha256,acceptance_sha256,independent_verifier,runner_profile,descriptor_digest) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13) ON CONFLICT(adoption_digest) DO NOTHING",params).map_err(|_| DATABASE)?;
        let matches: bool = tx.query_one("SELECT project_id=$2 AND project_snapshot_id=$3 AND task_ref=$4 AND client_request_id=$5 AND expected_head_digest=$6 AND artifact_ref=$7 AND acceptance_ref=$8 AND artifact_sha256=$9 AND acceptance_sha256=$10 AND independent_verifier=$11 AND runner_profile=$12 AND descriptor_digest=$13 FROM ONLY control_product.local_verified_result_evidence WHERE adoption_digest=$1",params).map_err(|_| DATABASE)?.get(0);
        if !matches {
            return Err(MISMATCH);
        }
        tx.commit()
            .map_err(|_| "LATTICE_LOCAL_RESULT_IMPORT_OUTCOME_UNKNOWN")?;
        Ok(
            json!({"schema":"lattice.local-result-import-receipt.v1","status":if inserted==0 {"REPLAYED"} else {"RECORDED"},
            "task_ref":self.adoption.task_ref().as_str(),"adoption_digest":self.adoption.result_digest().as_str(),
            "descriptor_digest":evidence.descriptor_digest().as_str(),"verification_log_sha256":evidence.acceptance_sha256().as_str()}),
        )
    }
}

fn retain_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(mut file) => {
            file.write_all(bytes).map_err(|_| REJECTED)?;
            file.sync_all().map_err(|_| REJECTED)
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            if file_bytes(path, 2 * 1024 * 1024)? == bytes {
                Ok(())
            } else {
                Err(MISMATCH)
            }
        }
        Err(_) => Err(REJECTED),
    }
}

fn run_node(node: &Path, workspace: &Path, test: &Path) -> Result<(bool, Vec<u8>)> {
    if !node.is_absolute() || !node.is_file() {
        return Err(REJECTED);
    }
    let mut command = Command::new(node);
    // The test process receives no Runtime or migrator credentials.
    command.env_clear();
    for key in [
        "SystemRoot",
        "WINDIR",
        "COMSPEC",
        "PATH",
        "TEMP",
        "TMP",
        "USERPROFILE",
        "HOMEDRIVE",
        "HOMEPATH",
    ] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command
        .current_dir(workspace)
        .args(["--test", "--test-reporter=tap"])
        .arg(Path::new(".").join(test.strip_prefix(workspace).map_err(|_| REJECTED)?))
        .env_remove("NODE_OPTIONS")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let mut child = command.spawn().map_err(|_| REJECTED)?;
    let (sender, receiver) = mpsc::channel();
    fn reader<R: Read + Send + 'static>(
        mut reader: R,
        index: usize,
        sender: mpsc::Sender<(usize, std::io::Result<Vec<u8>>)>,
    ) {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let result = reader
                .by_ref()
                .take(1024 * 1024 + 1)
                .read_to_end(&mut bytes)
                .map(|_| bytes);
            let _ = sender.send((index, result));
        });
    }
    reader(child.stdout.take().ok_or(REJECTED)?, 0, sender.clone());
    reader(child.stderr.take().ok_or(REJECTED)?, 1, sender);
    let until = Instant::now() + Duration::from_secs(60);
    loop {
        if let Some(status) = child.try_wait().map_err(|_| REJECTED)? {
            let mut streams = [Vec::new(), Vec::new()];
            for _ in 0..2 {
                let (index, result) = receiver
                    .recv_timeout(Duration::from_secs(1))
                    .map_err(|_| REJECTED)?;
                let bytes = result.map_err(|_| REJECTED)?;
                if bytes.len() > 1024 * 1024 {
                    return Err(REJECTED);
                }
                streams[index] = bytes;
            }
            let passed = status.success() && complete_tap_pass(&streams[0]);
            let mut output = b"--- stdout ---\n".to_vec();
            output.extend_from_slice(&streams[0]);
            output.extend_from_slice(b"\n--- stderr ---\n");
            output.extend_from_slice(&streams[1]);
            return Ok((passed, output));
        }
        if Instant::now() >= until {
            let _ = child.kill();
            let _ = child.wait();
            return Err("LATTICE_LOCAL_RESULT_VERIFICATION_TIMEOUT");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn complete_tap_pass(stdout: &[u8]) -> bool {
    let Ok(stdout) = std::str::from_utf8(stdout) else {
        return false;
    };
    let mut counts = [None; 4];
    for line in stdout.lines() {
        for (index, key) in ["# tests ", "# pass ", "# fail ", "# cancelled "]
            .iter()
            .enumerate()
        {
            if let Some(number) = line.strip_prefix(key) {
                if counts[index].is_some() {
                    return false;
                }
                let Ok(number) = number.parse::<u64>() else {
                    return false;
                };
                counts[index] = Some(number);
            }
        }
    }
    matches!(counts,[Some(tests),Some(pass),Some(0),Some(0)] if tests>0 && pass>0 && pass<=tests)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_tap_requires_completed_nonempty_success_and_rejects_spoofed_summary() {
        let good = b"TAP version 13\n1..3\n# tests 3\n# pass 3\n# fail 0\n# cancelled 0\n";
        assert!(complete_tap_pass(good));
        for failed in [
            "# tests 0\n# pass 0\n# fail 0\n# cancelled 0\n",
            "# tests 1\n# pass 0\n# fail 0\n# cancelled 0\n",
            "# tests 2\n# pass 1\n# fail 1\n# cancelled 0\n",
            "# tests 2\n# pass 1\n# fail 0\n# cancelled 1\n",
            "# tests 1\n# pass 1\n# fail 0\n",
            "# tests 1\n# tests 1\n# pass 1\n# fail 0\n# cancelled 0\n",
        ] {
            assert!(!complete_tap_pass(failed.as_bytes()));
        }
    }
}
