//! Trusted, local maintenance ingress for externally issued delivery receipts.
//! Hashes prove retained bytes and bindings, not the truth of a test narrative.
//! The issuer remains responsible for actually executing independent acceptance.
//! This module also rechecks Git, the active installation and physical artifacts.
//! Receipt argv is descriptive data and is never executed.

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use lattice_artifact_store::ExternalVerifiedResultEvidence;
use lattice_contracts::ContentDigest;
use lattice_task_ledger::{ExternalVerifiedResultAdoption, TaskSubmissionEnvelope};
use postgres::{Client, IsolationLevel};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

type Result<T> = std::result::Result<T, &'static str>;
const REJECTED: &str = "LATTICE_EXTERNAL_RESULT_IMPORT_REJECTED";
const MISMATCH: &str = "LATTICE_EXTERNAL_RESULT_EVIDENCE_MISMATCH";
const DATABASE: &str = "LATTICE_EXTERNAL_RESULT_IMPORT_DATABASE_UNAVAILABLE";
const MAX_JSON: u64 = 1024 * 1024;

pub(crate) struct ImportRequest {
    root: PathBuf,
    pub(crate) adoption: ExternalVerifiedResultAdoption,
}

pub(crate) struct VerifiedImport {
    pub(crate) adoption: ExternalVerifiedResultAdoption,
    pub(crate) evidence: ExternalVerifiedResultEvidence,
}

pub(crate) fn text<'a>(value: &'a Value, field: &str) -> Result<&'a str> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty() && text.len() <= 8192)
        .ok_or(REJECTED)
}

pub(crate) fn digest(value: &str) -> Result<ContentDigest> {
    ContentDigest::from_sha256(value).map_err(|_| REJECTED)
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub(crate) fn file_bytes(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let file = File::open(path).map_err(|_| REJECTED)?;
    if !file.metadata().map_err(|_| REJECTED)?.is_file() {
        return Err(REJECTED);
    }
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| REJECTED)?;
    if u64::try_from(bytes.len()).map_err(|_| REJECTED)? > limit {
        return Err(REJECTED);
    }
    Ok(bytes)
}

pub(crate) fn json_file(path: &Path) -> Result<Value> {
    serde_json::from_slice(&file_bytes(path, MAX_JSON)?).map_err(|_| REJECTED)
}

pub(crate) fn file_digest(path: &Path) -> Result<ContentDigest> {
    if !path.is_absolute() {
        return Err(REJECTED);
    }
    let mut file = File::open(path).map_err(|_| REJECTED)?;
    let metadata = file.metadata().map_err(|_| REJECTED)?;
    if !metadata.is_file() || metadata.len() > 2 * 1024 * 1024 * 1024 {
        return Err(REJECTED);
    }
    let mut hash = Sha256::new();
    let mut bytes = [0_u8; 64 * 1024];
    loop {
        let count = file.read(&mut bytes).map_err(|_| REJECTED)?;
        if count == 0 {
            break;
        }
        hash.update(&bytes[..count]);
    }
    digest(&hex(&hash.finalize()))
}

pub(crate) fn parse(path: &Path) -> Result<ImportRequest> {
    if !path.is_absolute() {
        return Err(REJECTED);
    }
    let value = json_file(path)?;
    let fields = [
        "schema",
        "evidence_root",
        "task_ref",
        "client_request_id",
        "expected_ledger_head_digest",
        "source_sha",
        "target_sha",
        "push_merge_receipt_ref",
        "deployment_receipt_ref",
        "deployment_artifact_ref",
        "independent_acceptance_ref",
        "protected_action_approval_refs",
    ];
    let object = value.as_object().ok_or(REJECTED)?;
    if object.len() != fields.len()
        || fields.iter().any(|key| !object.contains_key(*key))
        || text(&value, "schema")? != "lattice.external-result-import.v1"
    {
        return Err(REJECTED);
    }
    let declared_root = Path::new(text(&value, "evidence_root")?);
    if !declared_root.is_absolute() {
        return Err(REJECTED);
    }
    let root = fs::canonicalize(declared_root).map_err(|_| REJECTED)?;
    if !root.is_dir() {
        return Err(REJECTED);
    }
    let approvals = value["protected_action_approval_refs"]
        .as_array()
        .ok_or(REJECTED)?
        .iter()
        .map(|value| value.as_str().map(str::to_owned).ok_or(REJECTED))
        .collect::<Result<Vec<_>>>()?;
    let adoption = ExternalVerifiedResultAdoption::new(
        digest(text(&value, "task_ref")?)?,
        text(&value, "client_request_id")?,
        digest(text(&value, "expected_ledger_head_digest")?)?,
        text(&value, "source_sha")?,
        text(&value, "target_sha")?,
        text(&value, "push_merge_receipt_ref")?,
        text(&value, "deployment_receipt_ref")?,
        text(&value, "deployment_artifact_ref")?,
        text(&value, "independent_acceptance_ref")?,
        approvals,
    )
    .map_err(|_| REJECTED)?;
    Ok(ImportRequest { root, adoption })
}

impl ImportRequest {
    fn receipt(&self, reference: &str, kind: &str) -> Result<Value> {
        let expected = digest(reference.strip_prefix("evidence:sha256:").ok_or(REJECTED)?)?;
        let path = fs::canonicalize(self.root.join(format!("{}.json", expected.as_str())))
            .map_err(|_| REJECTED)?;
        if !path.starts_with(&self.root) {
            return Err(REJECTED);
        }
        let bytes = file_bytes(&path, MAX_JSON)?;
        if hex(&Sha256::digest(&bytes)) != expected.as_str() {
            return Err(MISMATCH);
        }
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| REJECTED)?;
        if text(&value, "schema")? != format!("lattice.external-result.{kind}.v1")
            || text(&value, "task_ref")? != self.adoption.task_ref().as_str()
            || text(&value, "target_sha")? != self.adoption.target_sha()
        {
            return Err(MISMATCH);
        }
        Ok(value)
    }

    fn command_receipt(&self, reference: &str) -> Result<()> {
        let value = self.receipt(reference, "command")?;
        let argv = value["argv"].as_array().ok_or(REJECTED)?;
        if value["exit_code"].as_i64() != Some(0)
            || argv.is_empty()
            || argv.len() > 64
            || argv.iter().any(|arg| {
                arg.as_str()
                    .is_none_or(|arg| arg.is_empty() || arg.len() > 8192)
            })
            || file_digest(Path::new(text(&value, "output_path")?))?
                != digest(text(&value, "output_sha256")?)?
        {
            return Err(MISMATCH);
        }
        Ok(())
    }

    pub(crate) fn verify(
        self,
        submission: &TaskSubmissionEnvelope,
        repository: &Path,
        git: &Path,
    ) -> Result<VerifiedImport> {
        if submission.task_ref() != self.adoption.task_ref()
            || submission.client_request_id() != self.adoption.client_request_id()
        {
            return Err(MISMATCH);
        }
        let push = self.receipt(self.adoption.push_merge_receipt_ref(), "push-merge")?;
        if text(&push, "source_sha")? != self.adoption.source_sha()
            || text(&push, "operation")? != "NON_FORCE_PUSH_MERGE"
        {
            return Err(MISMATCH);
        }
        self.command_receipt(text(&push, "command_evidence_ref")?)?;
        let remote_ref = text(&push, "remote_ref")?;
        if !remote_ref.starts_with("refs/heads/")
            || remote_ref.len() > 256
            || remote_ref.contains("..")
            || !remote_ref
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"/._-".contains(&byte))
        {
            return Err(REJECTED);
        }
        let origin = run_git(git, repository, &["config", "--get", "remote.origin.url"])?;
        if origin.trim() != text(&push, "remote_url")? {
            return Err(MISMATCH);
        }
        let remote = run_git(
            git,
            repository,
            &["ls-remote", "--exit-code", "--heads", "origin", remote_ref],
        )?;
        if remote.trim() != format!("{}\t{remote_ref}", self.adoption.target_sha()) {
            return Err(MISMATCH);
        }
        run_git(
            git,
            repository,
            &[
                "merge-base",
                "--is-ancestor",
                self.adoption.source_sha(),
                self.adoption.target_sha(),
            ],
        )?;
        run_git(
            git,
            repository,
            &[
                "diff",
                "--no-ext-diff",
                "--check",
                self.adoption.source_sha(),
                self.adoption.target_sha(),
            ],
        )?;

        let artifact = self.receipt(self.adoption.deployment_artifact_ref(), "artifact")?;
        let artifact_path = fs::canonicalize(text(&artifact, "path")?).map_err(|_| REJECTED)?;
        let artifact_digest = file_digest(&artifact_path)?;
        if artifact_digest != digest(text(&artifact, "sha256")?)? {
            return Err(MISMATCH);
        }
        let deployment = self.receipt(self.adoption.deployment_receipt_ref(), "deployment")?;
        if text(&deployment, "artifact_ref")? != self.adoption.deployment_artifact_ref() {
            return Err(MISMATCH);
        }
        let active = json_file(Path::new(text(&deployment, "active_install_path")?))?;
        let version_root =
            fs::canonicalize(text(&active, "version_root")?).map_err(|_| REJECTED)?;
        if text(&active, "schema_version")? != "lattice.control.desktop-active-install.v1"
            || text(&active, "source_commit")? != self.adoption.target_sha()
            || !artifact_path.starts_with(&version_root)
            || fs::canonicalize(text(&active, "executable")?).map_err(|_| REJECTED)?
                != artifact_path
        {
            return Err(MISMATCH);
        }
        let config_digest = file_digest(Path::new(text(&deployment, "config_path")?))?;
        if config_digest != digest(text(&deployment, "config_sha256")?)? {
            return Err(MISMATCH);
        }

        let acceptance = self.receipt(self.adoption.independent_acceptance_ref(), "acceptance")?;
        let verifier = text(&acceptance, "verifier_id")?;
        if verifier == text(&acceptance, "executor_id")? {
            return Err(MISMATCH);
        }
        let checks = acceptance["checks"].as_array().ok_or(REJECTED)?;
        if checks.is_empty() || checks.len() > 32 {
            return Err(REJECTED);
        }
        let mut names = BTreeSet::new();
        for check in checks {
            if !names.insert(text(check, "name")?) {
                return Err(REJECTED);
            }
            self.command_receipt(text(check, "evidence_ref")?)?;
        }
        let mut actions = BTreeSet::new();
        for reference in self.adoption.protected_action_approval_refs() {
            let approval = self.receipt(reference, "approval")?;
            if text(&approval, "authority")? != "USER" {
                return Err(MISMATCH);
            }
            let authorization =
                self.receipt(text(&approval, "authorization_ref")?, "authorization")?;
            text(&authorization, "source_reference")?;
            text(&authorization, "text")?;
            for action in approval["actions"].as_array().ok_or(REJECTED)? {
                actions.insert(action.as_str().ok_or(REJECTED)?.to_owned());
            }
        }
        if ["push", "merge", "deploy", "install"]
            .iter()
            .any(|action| !actions.contains(*action))
        {
            return Err(MISMATCH);
        }
        let evidence = ExternalVerifiedResultEvidence::new(
            submission.identity().project_id().clone(),
            submission.identity().project_snapshot_id().clone(),
            &self.adoption,
            self.adoption.target_sha(),
            artifact_digest,
            config_digest,
            verifier,
            true,
        )
        .map_err(|_| REJECTED)?;
        Ok(VerifiedImport {
            adoption: self.adoption,
            evidence,
        })
    }
}

pub(crate) fn run_git(git: &Path, repository: &Path, args: &[&str]) -> Result<String> {
    if !git.is_absolute() || !repository.is_absolute() {
        return Err(REJECTED);
    }
    let mut command = Command::new(git);
    command
        .arg("-C")
        .arg(repository)
        .args([
            "-c",
            "protocol.ext.allow=never",
            "-c",
            "credential.interactive=false",
            "-c",
            "core.fsmonitor=false",
        ])
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "never")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let mut child = command.spawn().map_err(|_| REJECTED)?;
    let stdout = child.stdout.take().ok_or(REJECTED)?;
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut output = Vec::new();
        let result = stdout.take(65537).read_to_end(&mut output).map(|_| output);
        let _ = sender.send(result);
    });
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(status) = child.try_wait().map_err(|_| REJECTED)? {
            if !status.success() {
                return Err(MISMATCH);
            }
            let bytes = receiver
                .recv_timeout(Duration::from_secs(1))
                .map_err(|_| REJECTED)?
                .map_err(|_| REJECTED)?;
            if bytes.len() > 65536 {
                return Err(REJECTED);
            }
            return String::from_utf8(bytes).map_err(|_| REJECTED);
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err("LATTICE_EXTERNAL_RESULT_GIT_TIMEOUT");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub(crate) fn digest_bytes(value: &ContentDigest) -> Vec<u8> {
    value
        .as_str()
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            u8::from_str_radix(std::str::from_utf8(pair).expect("digest ASCII"), 16)
                .expect("digest hex")
        })
        .collect()
}

/// Only a migrator connection may retain independently issued facts. No task
/// state is mutated here; the subsequent Runtime adoption owns that transaction.
pub(crate) fn retain(client: &mut Client, verified: &VerifiedImport) -> Result<Value> {
    let adoption = &verified.adoption;
    let evidence = &verified.evidence;
    let adoption_digest = digest_bytes(adoption.result_digest());
    let artifact_digest = digest_bytes(evidence.deployment_artifact_sha256());
    let config_digest = digest_bytes(evidence.config_command_sha256());
    let descriptor_digest = digest_bytes(evidence.descriptor_digest());
    let mut transaction = client
        .build_transaction()
        .isolation_level(IsolationLevel::Serializable)
        .start()
        .map_err(|_| DATABASE)?;
    transaction
        .batch_execute("SET LOCAL lock_timeout='5s'; SET LOCAL statement_timeout='30s'")
        .map_err(|_| DATABASE)?;
    let role = transaction.query_one("SELECT session_user = 'lattice_migrator_login' AND current_setting('role') = 'lattice_migrator'", &[])
        .map_err(|_| DATABASE)?;
    if !role.get::<_, bool>(0) {
        return Err(REJECTED);
    }
    let params: &[&(dyn postgres::types::ToSql + Sync)] = &[
        &adoption_digest,
        &evidence.project_id().as_str(),
        &evidence.project_snapshot_id().as_str(),
        &adoption.task_ref().as_str(),
        &adoption.source_sha(),
        &adoption.target_sha(),
        &evidence.remote_target_sha(),
        &adoption.push_merge_receipt_ref(),
        &adoption.deployment_receipt_ref(),
        &adoption.deployment_artifact_ref(),
        &adoption.independent_acceptance_ref(),
        &adoption.protected_action_approval_refs(),
        &artifact_digest,
        &config_digest,
        &evidence.independent_verifier(),
        &evidence.non_force_push_merge(),
        &descriptor_digest,
    ];
    let inserted = transaction.execute(
        "INSERT INTO control.external_verified_result_evidence (adoption_digest,project_id,project_snapshot_id,task_ref,source_sha,target_sha,remote_target_sha,push_merge_receipt_ref,deployment_receipt_ref,deployment_artifact_ref,independent_acceptance_ref,protected_action_approval_refs,deployment_artifact_sha256,config_command_sha256,independent_verifier,non_force_push_merge,descriptor_digest) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17) ON CONFLICT (adoption_digest) DO NOTHING",
        params,
    ).map_err(|_| DATABASE)?;
    let row = transaction.query_one(
        "SELECT project_id=$2 AND project_snapshot_id=$3 AND task_ref=$4 AND source_sha=$5 AND target_sha=$6 AND remote_target_sha=$7 AND push_merge_receipt_ref=$8 AND deployment_receipt_ref=$9 AND deployment_artifact_ref=$10 AND independent_acceptance_ref=$11 AND protected_action_approval_refs=$12 AND deployment_artifact_sha256=$13 AND config_command_sha256=$14 AND independent_verifier=$15 AND non_force_push_merge=$16 AND descriptor_digest=$17 FROM ONLY control.external_verified_result_evidence WHERE adoption_digest=$1 FOR SHARE",
        params,
    ).map_err(|_| DATABASE)?;
    if !row.get::<_, bool>(0) {
        return Err(MISMATCH);
    }
    transaction
        .commit()
        .map_err(|_| "LATTICE_EXTERNAL_RESULT_IMPORT_OUTCOME_UNKNOWN")?;
    Ok(json!({"schema":"lattice.external-result-import-receipt.v1",
        "status": if inserted == 0 { "REPLAYED" } else { "RECORDED" },
        "task_ref": adoption.task_ref().as_str(), "adoption_digest": adoption.result_digest().as_str(),
        "descriptor_digest": evidence.descriptor_digest().as_str()}))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Files(PathBuf);
    impl Files {
        fn new() -> Self {
            let mut random = [0_u8; 16];
            getrandom::fill(&mut random).unwrap();
            let path =
                std::env::temp_dir().join(format!("lattice-external-import-{}", hex(&random)));
            fs::create_dir(&path).unwrap();
            Self(fs::canonicalize(path).unwrap())
        }
        fn receipt(&self, value: &Value) -> String {
            let bytes = serde_json::to_vec(value).unwrap();
            let digest = hex(&Sha256::digest(&bytes));
            fs::write(self.0.join(format!("{digest}.json")), bytes).unwrap();
            format!("evidence:sha256:{digest}")
        }
    }
    impl Drop for Files {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn request(root: &Path) -> ImportRequest {
        let reference = |byte: char| format!("evidence:sha256:{}", byte.to_string().repeat(64));
        ImportRequest {
            root: root.to_owned(),
            adoption: ExternalVerifiedResultAdoption::new(
                digest(&"a".repeat(64)).unwrap(),
                "import-test",
                digest(&"b".repeat(64)).unwrap(),
                "1".repeat(40),
                "2".repeat(40),
                reference('3'),
                reference('4'),
                reference('5'),
                reference('6'),
                vec![reference('7')],
            )
            .unwrap(),
        }
    }

    #[test]
    fn external_import_command_receipts_bind_output_bytes_scope_and_exit_status() {
        let files = Files::new();
        let request = request(&files.0);
        let output = files.0.join("acceptance-output.txt");
        fs::write(&output, "2 tests passed").unwrap();
        let command = json!({"schema":"lattice.external-result.command.v1",
            "task_ref": "a".repeat(64), "target_sha":"2".repeat(40),
            "argv":["descriptive-only-command", "not-executed"], "exit_code":0,
            "output_path":output, "output_sha256":file_digest(&output).unwrap().as_str()});
        let reference = files.receipt(&command);
        request.command_receipt(&reference).unwrap();
        let mut failed = command.clone();
        failed["exit_code"] = json!(1);
        assert_eq!(
            request.command_receipt(&files.receipt(&failed)),
            Err(MISMATCH)
        );
        let mut substituted = command;
        substituted["task_ref"] = json!("c".repeat(64));
        assert_eq!(
            request.command_receipt(&files.receipt(&substituted)),
            Err(MISMATCH)
        );
        fs::write(&output, "changed after receipt issuance").unwrap();
        assert_eq!(request.command_receipt(&reference), Err(MISMATCH));
    }

    #[test]
    fn external_import_rejects_receipt_bytes_tampering_and_path_references() {
        let files = Files::new();
        let request = request(&files.0);
        let receipt = json!({"schema":"lattice.external-result.authorization.v1",
            "task_ref":"a".repeat(64), "target_sha":"2".repeat(40), "text":"Authorized"});
        let reference = files.receipt(&receipt);
        request.receipt(&reference, "authorization").unwrap();
        let path = files.0.join(format!(
            "{}.json",
            reference.strip_prefix("evidence:sha256:").unwrap()
        ));
        fs::write(path, "{}").unwrap();
        assert!(matches!(
            request.receipt(&reference, "authorization"),
            Err(MISMATCH)
        ));
        assert!(
            request
                .receipt("evidence:sha256:../../outside", "authorization")
                .is_err()
        );
        assert!(request.receipt("C:/outside.json", "authorization").is_err());
    }

    #[test]
    #[ignore = "requires an explicitly supplied intake in a disposable PostgreSQL cluster"]
    fn external_import_live_concurrent_adoption_and_fresh_replay() {
        use lattice_contracts::{
            DaemonEpoch, RuntimeAdmissionMode, RuntimeKind, StoreAuthorityHead,
            StoreAuthorityRevision, StoreDaemonInstanceId,
        };
        use lattice_postgres_store::{MigrationTarget, PostgresTaskLedger};
        use std::sync::{Arc, Barrier};
        let env = |name: &str| std::env::var(name).expect("explicit disposable fixture setting");
        assert_eq!(env("LATTICE_EXTERNAL_RESULT_LIVE"), "1");
        let port = env("LATTICE_TASK019_PORT").parse::<u16>().unwrap();
        assert!(![4317, 5432, 58743].contains(&port));
        let database = env("LATTICE_TASK_SUBMISSION_DATABASE");
        let run_id = env("LATTICE_TASK019_RUN_ID");
        let target = MigrationTarget::new(database.clone(), run_id).unwrap();
        let connect = |role: &str| {
            let mut config = postgres::Config::new();
            config
                .host("127.0.0.1")
                .port(port)
                .dbname(&database)
                .user(&format!("{role}_login"))
                .password(env("LATTICE_TASK019_PASSWORD"))
                .application_name("lattice-devos-task019");
            let mut client = config.connect(postgres::NoTls).expect("fixture connection");
            client.batch_execute(&format!("SET ROLE {role}")).unwrap();
            client
        };
        let mut ledger = PostgresTaskLedger::new(connect("lattice_runtime"), &target).unwrap();
        let task_ref = digest(&env("LATTICE_EXTERNAL_RESULT_LIVE_TASK_REF")).unwrap();
        let retained = ledger
            .load_submission_by_task_ref(&task_ref)
            .unwrap()
            .expect("fixture intake");
        let already_completed = retained.ledger().stream().events().len() == 2;
        assert!(matches!(retained.ledger().stream().events().len(), 1 | 2));
        let original_head = retained
            .ledger()
            .stream()
            .commands()
            .iter()
            .find(|record| {
                record.request().kind() == lattice_task_ledger::LedgerEventKind::TaskCreated
            })
            .unwrap()
            .receipt()
            .after()
            .head_digest()
            .clone();
        let reference = |byte: char| format!("evidence:sha256:{}", byte.to_string().repeat(64));
        let adoption = ExternalVerifiedResultAdoption::new(
            task_ref.clone(),
            retained.submission().client_request_id(),
            original_head,
            "1".repeat(40),
            "2".repeat(40),
            reference('3'),
            reference('4'),
            reference('5'),
            reference('6'),
            vec![reference('7')],
        )
        .unwrap();
        let authority = StoreAuthorityHead::new(
            RuntimeKind::Live,
            StoreDaemonInstanceId::new(env("LATTICE_STORE_DAEMON_INSTANCE_ID")).unwrap(),
            DaemonEpoch::new(env("LATTICE_STORE_DAEMON_EPOCH").parse().unwrap()).unwrap(),
            RuntimeAdmissionMode::Active,
            StoreAuthorityRevision::new(env("LATTICE_STORE_AUTHORITY_REVISION").parse().unwrap())
                .unwrap(),
            digest(&env("LATTICE_STORE_OBSERVATION_DIGEST")).unwrap(),
            digest(&env("LATTICE_STORE_AUTHORITY_HEAD_DIGEST")).unwrap(),
        )
        .unwrap();
        if std::env::var_os("LATTICE_EXTERNAL_RESULT_LIVE_RETAINED").is_none() {
            assert!(
                ledger
                    .execute_external_verified_result_adoption(
                        &adoption,
                        &authority,
                        "2026-09-05T01:00:00Z"
                    )
                    .is_err(),
                "missing independent evidence cannot complete the task"
            );
        }
        let evidence = ExternalVerifiedResultEvidence::new(
            retained.submission().identity().project_id().clone(),
            retained
                .submission()
                .identity()
                .project_snapshot_id()
                .clone(),
            &adoption,
            adoption.target_sha(),
            digest(&"8".repeat(64)).unwrap(),
            digest(&"9".repeat(64)).unwrap(),
            "disposable-independent-verifier",
            true,
        )
        .unwrap();
        let verified = VerifiedImport {
            adoption: adoption.clone(),
            evidence,
        };
        let mut migrator = connect("lattice_migrator");
        assert_eq!(
            retain(&mut migrator, &verified).unwrap()["status"],
            if std::env::var_os("LATTICE_EXTERNAL_RESULT_LIVE_RETAINED").is_some() {
                "REPLAYED"
            } else {
                "RECORDED"
            }
        );
        assert_eq!(
            retain(&mut migrator, &verified).unwrap()["status"],
            "REPLAYED"
        );
        let changed = VerifiedImport {
            adoption: adoption.clone(),
            evidence: ExternalVerifiedResultEvidence::new(
                verified.evidence.project_id().clone(),
                verified.evidence.project_snapshot_id().clone(),
                &adoption,
                adoption.target_sha(),
                digest(&"8".repeat(64)).unwrap(),
                digest(&"9".repeat(64)).unwrap(),
                "substituted-verifier",
                true,
            )
            .unwrap(),
        };
        assert!(
            retain(&mut migrator, &changed).is_err(),
            "changed import must roll back"
        );
        let first = PostgresTaskLedger::new(connect("lattice_runtime"), &target).unwrap();
        let second = PostgresTaskLedger::new(connect("lattice_runtime"), &target).unwrap();
        let barrier = Arc::new(Barrier::new(2));
        let workers = [first, second]
            .into_iter()
            .enumerate()
            .map(|(index, mut ledger)| {
                let adoption = adoption.clone();
                let authority = authority.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    ledger.execute_external_verified_result_adoption(
                        &adoption,
                        &authority,
                        if index == 0 {
                            "2026-09-05T01:00:01Z"
                        } else {
                            "2026-09-05T01:00:02Z"
                        },
                    )
                })
            })
            .collect::<Vec<_>>();
        let results = workers
            .into_iter()
            .map(|worker| worker.join().unwrap().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            results
                .iter()
                .filter(|result| !result.is_exact_retry())
                .count(),
            if already_completed { 0 } else { 1 }
        );
        assert_eq!(
            results[0].receipt().event_digest(),
            results[1].receipt().event_digest()
        );
        let mut fresh = PostgresTaskLedger::new(connect("lattice_runtime"), &target).unwrap();
        let completed = fresh
            .load_submission_by_task_ref(&task_ref)
            .unwrap()
            .unwrap();
        assert_eq!(completed.ledger().stream().events().len(), 2);
        assert_eq!(
            completed.ledger().stream().events()[1].subject_digest(),
            adoption.result_digest()
        );
        assert!(
            fresh
                .execute_external_verified_result_adoption(
                    &adoption,
                    &authority,
                    "2026-09-05T01:00:59Z"
                )
                .unwrap()
                .is_exact_retry()
        );
        // Inspect only this fixture's immutable binding; no baseline/schema tampering.
        let count: i64 = migrator.query_one(
            "SELECT count(*) FROM ONLY control.task_external_verified_result_adoptions WHERE adoption_digest=$1",
            &[&digest_bytes(adoption.result_digest())],
        ).unwrap().get(0);
        assert_eq!(count, 1);
    }
}
