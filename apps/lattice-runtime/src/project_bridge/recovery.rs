//! Operator-only branch identity recovery. No paths or authority supplied by requests.
use super::*;
use lattice_project_registry::{IdentityDrift, ReconciliationDecision, VerifiedRegistryState};
use serde_json::json;

/// Exact reviewed request. No automatic expected-head refresh is permitted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecoveryRequest {
    pub project_id: ProjectId,
    pub revision: u64,
    pub receipt_digest: ContentDigest,
    pub pending_digest: ContentDigest,
    pub restore_digest: Option<ContentDigest>,
}

impl RecoveryRequest {
    fn command_id(&self) -> ProjectBridgeResult<CommandId> {
        if let Some(proof) = &self.restore_digest {
            let value = format!(
                "{}:{}:{}:{}:{}",
                self.project_id.as_str(),
                self.revision,
                self.receipt_digest.as_str(),
                self.pending_digest.as_str(),
                proof.as_str()
            );
            let digest = bridge_digest(
                "lattice.project-bridge.restore-recovery",
                &CanonicalValue::String(value),
            )?;
            return CommandId::new(format!("restore-recovery-{}", digest.as_str()))
                .map_err(|_| bridge_error(ProjectBridgeErrorKind::InvalidSelector));
        }
        let digest = bridge_digest(
            "lattice.project-bridge.branch-recovery",
            &CanonicalValue::Object(vec![
                (
                    "project_id".into(),
                    CanonicalValue::String(self.project_id.as_str().into()),
                ),
                (
                    "revision".into(),
                    CanonicalValue::String(self.revision.to_string()),
                ),
                (
                    "receipt_digest".into(),
                    CanonicalValue::String(self.receipt_digest.as_str().into()),
                ),
                (
                    "pending_digest".into(),
                    CanonicalValue::String(self.pending_digest.as_str().into()),
                ),
            ]),
        )?;
        CommandId::new(format!("branch-recovery-{}", digest.as_str()))
            .map_err(|_| bridge_error(ProjectBridgeErrorKind::InvalidSelector))
    }
}

fn branch_only(current: &RegistryProjectProjection) -> bool {
    current.project_class() == ProjectClass::UserProject
        && current.authority().lifecycle() == ProjectLifecycle::ReconciliationRequired
        && !current.drift().is_empty()
        && current.drift().iter().all(|d| {
            matches!(
                d,
                IdentityDrift::PrimaryRefName | IdentityDrift::PrimaryRefStorage
            )
        })
}

fn prepare(
    state: &VerifiedRegistryState,
    request: &RecoveryRequest,
    fresh: &RepositoryObservation,
) -> ProjectBridgeResult<RegistryCommand> {
    let current = state
        .project(&request.project_id)
        .ok_or_else(|| bridge_error(ProjectBridgeErrorKind::ProjectNotFound))?;
    let restore_eligible = request.restore_digest.is_some()
        && current.project_class() == ProjectClass::UserProject
        && current.authority().lifecycle() == ProjectLifecycle::ReconciliationRequired
        && !current.drift().is_empty()
        && !current.drift().contains(&IdentityDrift::PrimaryRefName);
    if !(if request.restore_digest.is_some() {
        restore_eligible
    } else {
        branch_only(current)
    }) {
        return Err(bridge_error(ProjectBridgeErrorKind::ProjectInactive));
    }
    let head = current.authority().head();
    if head.registry_revision() != request.revision
        || head.receipt_digest() != &request.receipt_digest
    {
        return Err(bridge_error(
            ProjectBridgeErrorKind::ProjectRegistryConflict,
        ));
    }
    let pending = current
        .pending_observation()
        .ok_or_else(|| bridge_error(ProjectBridgeErrorKind::ProjectRegistryRejected))?;
    if pending.digest() != &request.pending_digest || pending != fresh {
        return Err(bridge_error(ProjectBridgeErrorKind::ProjectIdentityChanged));
    }
    Ok(RegistryCommand::reconcile(
        request.command_id()?,
        request.project_id.clone(),
        head,
        pending.clone(),
        if restore_eligible && current.drift() == [IdentityDrift::CanonicalRoot] {
            ReconciliationDecision::AcceptMove
        } else {
            ReconciliationDecision::AcceptIdentityChange
        },
        request
            .restore_digest
            .as_ref()
            .unwrap_or(&request.pending_digest)
            .clone(),
    ))
}

// Replay returns the retained result, never interprets it as current ACTIVE authority.
fn replay<'a>(
    state: &'a VerifiedRegistryState,
    request: &RecoveryRequest,
) -> ProjectBridgeResult<Option<&'a lattice_project_registry::RegistryCommandReceipt>> {
    let id = request.command_id()?;
    let retained = state
        .commands()
        .values()
        .find(|record| record.command().command_id() == &id);
    let Some(record) = retained else {
        return Ok(None);
    };
    match record.command() {
        RegistryCommand::Reconcile {
            project_id,
            expected_head,
            observation,
            decision,
            evidence_digest,
            ..
        } if project_id == &request.project_id
            && expected_head.project_id() == &request.project_id
            && expected_head.registry_revision() == request.revision
            && expected_head.receipt_digest() == &request.receipt_digest
            && observation.digest() == &request.pending_digest
            && (*decision == ReconciliationDecision::AcceptIdentityChange
                || (request.restore_digest.is_some()
                    && *decision == ReconciliationDecision::AcceptMove))
            && evidence_digest
                == request
                    .restore_digest
                    .as_ref()
                    .unwrap_or(&request.pending_digest) =>
        {
            Ok(Some(record.receipt()))
        }
        _ => Err(bridge_error(
            ProjectBridgeErrorKind::ProjectRegistryRejected,
        )),
    }
}

fn projection(current: &RegistryProjectProjection, fresh: &RepositoryObservation) -> Value {
    let head = current.authority().head();
    json!({
        "project_id": head.project_id().as_str(),
        "registry_revision": head.registry_revision(),
        "receipt_digest": head.receipt_digest().as_str(),
        "lifecycle": head.lifecycle().as_str(),
        "active": head.lifecycle() == ProjectLifecycle::Active,
        "accepted_observation_digest": current.observation().digest().as_str(),
        "accepted_ref": current.observation().primary_branch().reference(),
        "pending_observation_digest": current.pending_observation().map(|p| p.digest().as_str()),
        "pending_ref": current.pending_observation().map(|p| p.primary_branch().reference()),
        "fresh_observation_digest": fresh.digest().as_str(),
        "fresh_matches_accepted": fresh == current.observation(),
        "fresh_matches_pending": current.pending_observation() == Some(fresh),
        "branch_only_recovery_eligible": branch_only(current) && current.pending_observation() == Some(fresh),
        "drift": current.drift().iter().map(|d| d.as_str()).collect::<Vec<_>>()
    })
}

pub(crate) fn run(
    database: &DeliveryDatabaseBinding,
    password: &str,
    project_id: &ProjectId,
    request: Option<&RecoveryRequest>,
) -> ProjectBridgeResult<Value> {
    run_with_proof(database, password, project_id, request, None)
}

pub(crate) fn run_with_proof(
    database: &DeliveryDatabaseBinding,
    password: &str,
    project_id: &ProjectId,
    request: Option<&RecoveryRequest>,
    restore_proof: Option<&Path>,
) -> ProjectBridgeResult<Value> {
    let deadline = Instant::now() + Duration::from_secs(30);
    let origin = match env::var(CONTROL_ORIGIN_ENV) {
        Ok(value) => value,
        Err(env::VarError::NotPresent) => DEFAULT_CONTROL_ORIGIN.into(),
        Err(_) => return Err(bridge_error(ProjectBridgeErrorKind::ControlConfiguration)),
    };
    let origin = parse_control_origin(&origin)
        .ok_or_else(|| bridge_error(ProjectBridgeErrorKind::ControlConfiguration))?;
    let selector = ProjectSelector::new(Some(project_id.as_str()), None)?;
    let state = control_get_json(origin, "/api/state", deadline)?;
    let selected = select_catalog_project(&parse_catalog_state(&state)?, &selector)?.clone();
    let detail_path = format!("/api/projects/{}", selected.id);
    let detail_value = control_get_json(origin, &detail_path, deadline)?;
    let detail = parse_catalog_detail(&detail_value, Some(&selected))?;
    let fresh = inspect_repository(
        &detail.canonical_path,
        &configured_git_executable()?,
        deadline,
    )?;
    let again = control_get_json(origin, "/api/state", deadline)?;
    verify_catalog_replay(&state, &again, &selector, &selected)?;
    if control_get_json(origin, &detail_path, deadline)? != detail_value {
        return Err(bridge_error(ProjectBridgeErrorKind::ProjectIdentityChanged));
    }
    let target = MigrationTarget::new(database.database_name(), database.run_id())
        .map_err(|_| bridge_error(ProjectBridgeErrorKind::ProjectRegistryRejected))?;
    let client = connect_fixed_runtime_client(database, password, deadline)
        .map_err(|_| bridge_error(ProjectBridgeErrorKind::ProjectRegistryUnavailable))?;
    let mut registry = PostgresProjectRegistry::new(client, &target).map_err(map_registry_error)?;
    let loaded = registry.load().map_err(map_registry_error)?;
    let current = loaded
        .state()
        .project(project_id)
        .ok_or_else(|| bridge_error(ProjectBridgeErrorKind::ProjectNotFound))?;
    let Some(request) = request else {
        return Ok(
            json!({"schema_version":"lattice.project-registry.recovery.v1", "status":"INSPECTED",
                "registry_checkpoint_digest": loaded.state().checkpoint().checkpoint_digest().as_str(),
                "registry_command_count": loaded.state().commands().len(), "current":projection(current, &fresh)}),
        );
    };
    if &request.project_id != project_id {
        return Err(bridge_error(ProjectBridgeErrorKind::InvalidSelector));
    }
    if request.restore_digest.is_some() != restore_proof.is_some() {
        return Err(bridge_error(ProjectBridgeErrorKind::InvalidSelector));
    }
    if let (Some(path), Some(hash)) = (restore_proof, request.restore_digest.as_ref()) {
        authorize_restore(database, project_id, path, hash, Some(request))?;
    }
    if let Some(receipt) = replay(loaded.state(), request)? {
        return Ok(
            json!({"schema_version":"lattice.project-registry.recovery.v1", "status":"REPLAYED",
            "registry_checkpoint_digest": loaded.state().checkpoint().checkpoint_digest().as_str(),
            "registry_command_count": loaded.state().commands().len(),
            "command_id":request.command_id()?.as_str(), "historical_receipt_digest":receipt.result_digest().as_str(),
            "historical_outcome":format!("{:?}",receipt.outcome()), "current":projection(current, &fresh)}),
        );
    }
    if let Some(path) = restore_proof {
        verify_restore_proof(path, request, current, &detail.canonical_path, deadline)?;
    }
    let command = prepare(loaded.state(), request, &fresh)?;
    // DB loading can take time. Recheck the selected physical repository and
    // locator before committing; never replace the reviewed pending observation.
    if inspect_repository(
        &detail.canonical_path,
        &configured_git_executable()?,
        deadline,
    )? != fresh
        || control_get_json(origin, &detail_path, deadline)? != detail_value
    {
        return Err(bridge_error(ProjectBridgeErrorKind::ProjectIdentityChanged));
    }
    if let Some(path) = restore_proof {
        verify_restore_proof(path, request, current, &detail.canonical_path, deadline)?;
    }
    let authority = crate::composition::configured_store_authority()
        .map_err(|_| bridge_error(ProjectBridgeErrorKind::ProjectRegistryRejected))?;
    let execution = registry
        .execute(command, authority)
        .map_err(map_registry_error)?;
    match execution.semantic_receipt().outcome() {
        RegistryCommandOutcome::Applied => {}
        RegistryCommandOutcome::Denied(d) => return Err(map_registry_denial(&d)),
        RegistryCommandOutcome::Blocked(_) => {
            return Err(bridge_error(
                ProjectBridgeErrorKind::ProjectIdentityCollision,
            ));
        }
    }
    let reloaded = registry.load().map_err(map_registry_error)?;
    let current = reloaded
        .state()
        .project(project_id)
        .ok_or_else(|| bridge_error(ProjectBridgeErrorKind::ProjectRegistryRejected))?;
    // Both receipt and current projection are reported: a concurrent later command
    // must not be hidden by the historical success of this exact request.
    Ok(
        json!({"schema_version":"lattice.project-registry.recovery.v1", "status":if execution.is_exact_retry(){"REPLAYED"}else{"APPLIED"},
        "registry_checkpoint_digest": reloaded.state().checkpoint().checkpoint_digest().as_str(),
        "registry_command_count": reloaded.state().commands().len(),
        "command_id":request.command_id()?.as_str(), "historical_receipt_digest":execution.semantic_receipt().result_digest().as_str(),
        "historical_outcome":"Applied", "current":projection(current, &fresh)}),
    )
}

pub(crate) fn observe_restore(
    database: &DeliveryDatabaseBinding,
    password: &str,
    project_id: &ProjectId,
    proof: &Path,
    proof_digest: &ContentDigest,
) -> ProjectBridgeResult<Value> {
    let journal = authorize_restore(database, project_id, proof, proof_digest, None)?;
    let deadline = Instant::now() + Duration::from_secs(30);
    let origin = parse_control_origin(DEFAULT_CONTROL_ORIGIN)
        .ok_or_else(|| bridge_error(ProjectBridgeErrorKind::ControlConfiguration))?;
    let value = control_get_json(origin, "/api/state", deadline)?;
    let selector = ProjectSelector::new(Some(project_id.as_str()), None)?;
    let selected = select_catalog_project(&parse_catalog_state(&value)?, &selector)?.clone();
    let detail_path = format!("/api/projects/{}", selected.id);
    let detail_value = control_get_json(origin, &detail_path, deadline)?;
    let detail = parse_catalog_detail(&detail_value, Some(&selected))?;
    let fresh = inspect_repository(
        &detail.canonical_path,
        &configured_git_executable()?,
        deadline,
    )?;
    let target = MigrationTarget::new(database.database_name(), database.run_id())
        .map_err(|_| bridge_error(ProjectBridgeErrorKind::ProjectRegistryRejected))?;
    let client = connect_fixed_runtime_client(database, password, deadline)
        .map_err(|_| bridge_error(ProjectBridgeErrorKind::ProjectRegistryUnavailable))?;
    let mut registry = PostgresProjectRegistry::new(client, &target).map_err(map_registry_error)?;
    let loaded = registry.load().map_err(map_registry_error)?;
    let current = loaded
        .state()
        .project(project_id)
        .ok_or_else(|| bridge_error(ProjectBridgeErrorKind::ProjectNotFound))?;
    let original = &journal["original_registry"][project_id.as_str()];
    if original["accepted_observation_digest"] != current.observation().digest().as_str() {
        return Err(bridge_error(ProjectBridgeErrorKind::ProjectIdentityChanged));
    }
    let request = RecoveryRequest {
        project_id: project_id.clone(),
        revision: current.authority().head().registry_revision(),
        receipt_digest: current.authority().head().receipt_digest().clone(),
        pending_digest: fresh.digest().clone(),
        restore_digest: Some(proof_digest.clone()),
    };
    verify_restore_proof(proof, &request, current, &detail.canonical_path, deadline)?;
    if current.pending_observation() == Some(&fresh) {
        return Ok(json!({"status":"OBSERVED","current":projection(current, &fresh)}));
    }
    if original["registry_revision"] != request.revision
        || original["receipt_digest"] != request.receipt_digest.as_str()
        || current.authority().lifecycle() != ProjectLifecycle::Active
    {
        return Err(bridge_error(
            ProjectBridgeErrorKind::ProjectRegistryConflict,
        ));
    }
    if inspect_repository(
        &detail.canonical_path,
        &configured_git_executable()?,
        deadline,
    )? != fresh
        || control_get_json(origin, &detail_path, deadline)? != detail_value
    {
        return Err(bridge_error(ProjectBridgeErrorKind::ProjectIdentityChanged));
    }
    let head = current.authority().head();
    let command = RegistryCommand::observe(
        registry_command_id("observe", project_id, &fresh, Some(&head))?,
        project_id.clone(),
        head,
        fresh.clone(),
    );
    registry
        .execute(
            command,
            crate::composition::configured_store_authority()
                .map_err(|_| bridge_error(ProjectBridgeErrorKind::ProjectRegistryRejected))?,
        )
        .map_err(map_registry_error)?;
    let loaded = registry.load().map_err(map_registry_error)?;
    let current = loaded
        .state()
        .project(project_id)
        .ok_or_else(|| bridge_error(ProjectBridgeErrorKind::ProjectNotFound))?;
    if current.pending_observation() != Some(&fresh) {
        return Err(bridge_error(
            ProjectBridgeErrorKind::ProjectRegistryRejected,
        ));
    }
    Ok(json!({"status":"OBSERVED","current":projection(current, &fresh)}))
}

fn authorize_restore(
    database: &DeliveryDatabaseBinding,
    project_id: &ProjectId,
    proof: &Path,
    proof_digest: &ContentDigest,
    request: Option<&RecoveryRequest>,
) -> ProjectBridgeResult<Value> {
    let rejected = || bridge_error(ProjectBridgeErrorKind::ProjectIdentityChanged);
    let root = PathBuf::from(env::var_os("LATTICE_CUSTOMER_RESTORE_ROOT").ok_or_else(rejected)?);
    let catalog = PathBuf::from(env::var_os(CUSTOMER_CATALOG_ENV).ok_or_else(rejected)?);
    authorize_restore_at(
        &root,
        &catalog,
        database,
        project_id,
        proof,
        proof_digest,
        request,
    )
}

fn authorize_restore_at(
    root: &Path,
    catalog: &Path,
    database: &DeliveryDatabaseBinding,
    project_id: &ProjectId,
    proof: &Path,
    proof_digest: &ContentDigest,
    request: Option<&RecoveryRequest>,
) -> ProjectBridgeResult<Value> {
    use sha2::{Digest, Sha256};
    let rejected = || bridge_error(ProjectBridgeErrorKind::ProjectIdentityChanged);
    if !root.is_absolute()
        || !root.join("restore.in-progress").is_file()
        || catalog != root.join("projects.json")
        || proof
            != root
                .join("restore-proofs")
                .join(format!("{}.json", project_id.as_str()))
    {
        return Err(rejected());
    }
    let public = bounded_restore_file(&root.join("installation.json"))?;
    let hash: String = Sha256::digest(&public)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let credentials = unseal_restore_file(&root.join("credentials.dpapi"))?;
    let journal = unseal_restore_file(&root.join("restore.pending.dpapi"))?;
    let config: Value = serde_json::from_slice(&public).map_err(|_| rejected())?;
    if credentials["installation_sha256"] != hash
        || journal["installation_sha256"] != hash
        || journal["schema"] != "lattice.customer-restore.v1"
        || config["schema"] != "lattice.customer-runtime.v1"
        || journal["root"].as_str().map(Path::new) != Some(root)
        || config["root"].as_str().map(Path::new) != Some(root)
        || config["run_id"] != database.run_id()
        || config["port"] != database.port()
        || journal["proofs"][project_id.as_str()] != proof_digest.as_str()
    {
        return Err(rejected());
    }
    let catalog = bounded_restore_file(&root.join("projects.json"))?;
    let catalog_hash: String = Sha256::digest(catalog)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if journal["catalog_sha256"] != catalog_hash {
        return Err(rejected());
    }
    if let Some(request) = request {
        let expected = json!([
            "--expected-revision",
            request.revision.to_string(),
            "--expected-receipt-digest",
            request.receipt_digest.as_str(),
            "--pending-observation-digest",
            request.pending_digest.as_str(),
            "--restore-proof",
            proof.to_str().ok_or_else(rejected)?,
            "--restore-proof-sha256",
            proof_digest.as_str()
        ]);
        if journal["requests"][project_id.as_str()] != expected {
            return Err(rejected());
        }
    }
    Ok(journal)
}

fn bounded_restore_file(path: &Path) -> ProjectBridgeResult<Vec<u8>> {
    let rejected = || bridge_error(ProjectBridgeErrorKind::ProjectIdentityChanged);
    let metadata = fs::symlink_metadata(path).map_err(|_| rejected())?;
    if !metadata.file_type().is_file() || metadata.len() > 32 * 1024 * 1024 {
        return Err(rejected());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(rejected());
        }
    }
    fs::read(path).map_err(|_| rejected())
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn unseal_restore_file(path: &Path) -> ProjectBridgeResult<Value> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CryptUnprotectData};
    let mut encrypted = bounded_restore_file(path)?;
    let input = CRYPT_INTEGER_BLOB {
        cbData: encrypted.len() as u32,
        pbData: encrypted.as_mut_ptr(),
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    if unsafe {
        CryptUnprotectData(
            &input,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            1,
            &mut output,
        )
    } == 0
    {
        return Err(bridge_error(ProjectBridgeErrorKind::ProjectIdentityChanged));
    }
    let result = if output.cbData > 32 * 1024 * 1024 {
        Err(bridge_error(ProjectBridgeErrorKind::ProjectIdentityChanged))
    } else {
        let plaintext =
            unsafe { std::slice::from_raw_parts_mut(output.pbData, output.cbData as usize) };
        let value = serde_json::from_slice(plaintext)
            .map_err(|_| bridge_error(ProjectBridgeErrorKind::ProjectIdentityChanged));
        plaintext.fill(0);
        value
    };
    unsafe {
        LocalFree(output.pbData.cast());
    }
    result
}

#[cfg(not(windows))]
fn unseal_restore_file(_path: &Path) -> ProjectBridgeResult<Value> {
    Err(bridge_error(ProjectBridgeErrorKind::ProjectIdentityChanged))
}

fn verify_restore_proof(
    path: &Path,
    request: &RecoveryRequest,
    current: &RegistryProjectProjection,
    root: &Path,
    deadline: Instant,
) -> ProjectBridgeResult<()> {
    use sha2::{Digest, Sha256};
    let rejected = || bridge_error(ProjectBridgeErrorKind::ProjectIdentityChanged);
    let metadata = fs::symlink_metadata(path).map_err(|_| rejected())?;
    if !metadata.file_type().is_file() || metadata.len() > 32 * 1024 * 1024 {
        return Err(rejected());
    }
    let bytes = fs::read(path).map_err(|_| rejected())?;
    let hash: String = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if request.restore_digest.as_ref().map(ContentDigest::as_str) != Some(hash.as_str()) {
        return Err(rejected());
    }
    let proof: Value = serde_json::from_slice(&bytes).map_err(|_| rejected())?;
    if proof["schema"] != "lattice.customer-project-restore-proof.v1"
        || proof["project_id"] != request.project_id.as_str()
        || proof["accepted_observation_digest"] != current.observation().digest().as_str()
    {
        return Err(rejected());
    }
    let expected = proof["files"].as_object().ok_or_else(rejected)?;
    if expected.is_empty() || expected.len() > 100_000 {
        return Err(rejected());
    }
    let mut observed = serde_json::Map::new();
    let mut directories = vec![root.to_owned()];
    let mut total = 0_u64;
    while let Some(directory) = directories.pop() {
        ensure_before(deadline)?;
        for entry in fs::read_dir(directory).map_err(|_| rejected())? {
            let entry = entry.map_err(|_| rejected())?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).map_err(|_| rejected())?;
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if metadata.file_attributes() & 0x400 != 0 {
                    return Err(rejected());
                }
            }
            if metadata.file_type().is_dir() {
                directories.push(path);
                if directories.len() > 100_000 {
                    return Err(rejected());
                }
            } else if metadata.file_type().is_file() {
                total = total.checked_add(metadata.len()).ok_or_else(rejected)?;
                if total > 10 * 1024 * 1024 * 1024 || observed.len() >= 100_000 {
                    return Err(rejected());
                }
                let name = path
                    .strip_prefix(root)
                    .map_err(|_| rejected())?
                    .to_str()
                    .ok_or_else(rejected)?
                    .replace('\\', "/");
                let mut file = File::open(&path).map_err(|_| rejected())?;
                let mut hasher = Sha256::new();
                let mut buffer = [0_u8; 65536];
                loop {
                    ensure_before(deadline)?;
                    let count = file.read(&mut buffer).map_err(|_| rejected())?;
                    if count == 0 {
                        break;
                    }
                    hasher.update(&buffer[..count]);
                }
                let hash: String = hasher
                    .finalize()
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect();
                observed.insert(name, json!({"sha256":hash,"bytes":metadata.len()}));
            } else {
                return Err(rejected());
            }
        }
    }
    if &observed != expected {
        return Err(rejected());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use lattice_project_registry::FakeProjectRegistry;

    #[cfg(windows)]
    #[allow(unsafe_code)]
    fn seal_test(value: &Value) -> Vec<u8> {
        use windows_sys::Win32::Foundation::LocalFree;
        use windows_sys::Win32::Security::Cryptography::{CRYPT_INTEGER_BLOB, CryptProtectData};
        let mut bytes = serde_json::to_vec(value).unwrap();
        let input = CRYPT_INTEGER_BLOB {
            cbData: bytes.len() as u32,
            pbData: bytes.as_mut_ptr(),
        };
        let mut output = CRYPT_INTEGER_BLOB {
            cbData: 0,
            pbData: std::ptr::null_mut(),
        };
        assert_ne!(
            unsafe {
                CryptProtectData(
                    &input,
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null(),
                    std::ptr::null(),
                    1,
                    &mut output,
                )
            },
            0
        );
        let result =
            unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec();
        unsafe {
            LocalFree(output.pbData.cast());
        }
        result
    }

    #[test]
    #[cfg(windows)]
    fn restore_authority_rejects_missing_journal_and_changed_bindings() {
        use sha2::{Digest, Sha256};
        let hex = |bytes: &[u8]| -> String {
            Sha256::digest(bytes)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect()
        };
        let root =
            std::env::temp_dir().join(format!("lattice-restore-auth-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        fs::write(root.join("restore.in-progress"), b"gate").unwrap();
        fs::write(root.join("projects.json"), b"fixture catalog").unwrap();
        let project = ProjectId::new("restore-project").unwrap();
        let proof = root.join("restore-proofs/restore-project.json");
        let binding = DeliveryDatabaseBinding::new("127.0.0.1", 54321, "a".repeat(32)).unwrap();
        let request = RecoveryRequest {
            project_id: project.clone(),
            revision: 2,
            receipt_digest: digest('b'),
            pending_digest: digest('c'),
            restore_digest: Some(digest('d')),
        };
        for case in 0..7 {
            let mut config = json!({"schema":"lattice.customer-runtime.v1","root":root,"run_id":"a".repeat(32),"port":54321});
            if case == 2 {
                config["run_id"] = json!("f".repeat(32));
            }
            if case == 3 {
                config["port"] = json!(54322);
            }
            let bytes = serde_json::to_vec(&config).unwrap();
            let hash = hex(&bytes);
            fs::write(root.join("installation.json"), bytes).unwrap();
            fs::write(
                root.join("credentials.dpapi"),
                seal_test(&json!({"installation_sha256":hash,"password":"fixture-only"})),
            )
            .unwrap();
            let mut journal = json!({"schema":"lattice.customer-restore.v1","root":root,"installation_sha256":hash,
                "catalog_sha256":hex(b"fixture catalog"),"proofs":{"restore-project":digest('d').as_str()},
                "requests":{"restore-project":["--expected-revision","2","--expected-receipt-digest",digest('b').as_str(),
                "--pending-observation-digest",digest('c').as_str(),"--restore-proof",proof,
                "--restore-proof-sha256",digest('d').as_str()]}});
            if case == 4 {
                journal["catalog_sha256"] = json!("0".repeat(64));
            }
            if case == 5 {
                journal["proofs"]["restore-project"] = json!(digest('e').as_str());
            }
            if case == 6 {
                journal["requests"]["restore-project"][1] = json!("3");
            }
            fs::write(root.join("restore.pending.dpapi"), seal_test(&journal)).unwrap();
            if case == 1 {
                fs::remove_file(root.join("restore.pending.dpapi")).unwrap();
            }
            let result = authorize_restore_at(
                &root,
                &root.join("projects.json"),
                &binding,
                &project,
                &proof,
                &digest('d'),
                Some(&request),
            );
            assert_eq!(result.is_ok(), case == 0, "case {case}");
        }
        for name in [
            "restore.in-progress",
            "projects.json",
            "installation.json",
            "credentials.dpapi",
            "restore.pending.dpapi",
        ] {
            fs::remove_file(root.join(name)).unwrap();
        }
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn restore_requires_exact_head_and_preserves_original_project_identity() {
        let pending = RepositoryObservation::new(
            r"C:\restored\project",
            digest('a'),
            digest('b'),
            digest('c'),
            GitRefIdentity::new("refs/heads/main", digest('d')).unwrap(),
        )
        .unwrap();
        let (mut registry, mut request) = fixture(pending.clone());
        assert!(prepare(registry.verified_state(), &request, &pending).is_err());
        request.restore_digest = Some(digest('e'));
        let command = prepare(registry.verified_state(), &request, &pending).unwrap();
        let receipt = registry.execute(command).unwrap();
        assert_eq!(receipt.outcome(), RegistryCommandOutcome::Applied);
        assert_eq!(
            replay(registry.verified_state(), &request).unwrap(),
            Some(&receipt)
        );
        let current = registry
            .verified_state()
            .project(&request.project_id)
            .unwrap();
        assert_eq!(current.authority().head().project_id(), &request.project_id);
        let (registry, mut stale) = fixture(pending.clone());
        stale.restore_digest = Some(digest('e'));
        stale.revision -= 1;
        assert!(prepare(registry.verified_state(), &stale, &pending).is_err());
        let different_branch = observation("refs/heads/other");
        let (registry, mut request) = fixture(different_branch.clone());
        request.restore_digest = Some(digest('e'));
        assert!(prepare(registry.verified_state(), &request, &different_branch).is_err());
    }

    #[test]
    fn restore_proof_rejects_changed_files_and_wrong_original_observation() {
        use sha2::{Digest, Sha256};
        let hex = |bytes: &[u8]| -> String {
            Sha256::digest(bytes)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect()
        };
        let directory =
            std::env::temp_dir().join(format!("lattice-restore-proof-{}", std::process::id()));
        fs::create_dir(&directory).unwrap();
        let root = directory.join("project");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("customer.py"), b"fixture").unwrap();
        let (registry, mut request) = fixture(observation("refs/heads/other"));
        let current = registry
            .verified_state()
            .project(&request.project_id)
            .unwrap();
        let path = directory.join("proof.json");
        let mut proof = json!({"schema":"lattice.customer-project-restore-proof.v1","project_id":request.project_id.as_str(),
            "accepted_observation_digest":current.observation().digest().as_str(),"files":{"customer.py":{"sha256":hex(b"fixture"),"bytes":7}}});
        let bytes = serde_json::to_vec(&proof).unwrap();
        fs::write(&path, &bytes).unwrap();
        request.restore_digest = Some(ContentDigest::from_sha256(hex(&bytes)).unwrap());
        let deadline = Instant::now() + Duration::from_secs(10);
        verify_restore_proof(&path, &request, current, &root, deadline).unwrap();
        fs::write(root.join("customer.py"), b"changed").unwrap();
        assert!(verify_restore_proof(&path, &request, current, &root, deadline).is_err());
        fs::write(root.join("customer.py"), b"fixture").unwrap();
        proof["accepted_observation_digest"] = json!(digest('f').as_str());
        let bytes = serde_json::to_vec(&proof).unwrap();
        fs::write(&path, &bytes).unwrap();
        request.restore_digest = Some(ContentDigest::from_sha256(hex(&bytes)).unwrap());
        assert!(verify_restore_proof(&path, &request, current, &root, deadline).is_err());
        fs::remove_file(root.join("customer.py")).unwrap();
        fs::remove_dir(root).unwrap();
        fs::remove_file(path).unwrap();
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn branch_recovery_replay_rejects_command_id_substitution() {
        let pending = observation("refs/heads/feature");
        let (mut registry, request) = fixture(pending.clone());
        registry
            .execute(RegistryCommand::reconcile(
                request.command_id().unwrap(),
                request.project_id.clone(),
                registry.current_head(&request.project_id).unwrap(),
                pending,
                ReconciliationDecision::AcceptIdentityChange,
                digest('a'),
            ))
            .unwrap();
        assert_eq!(
            replay(registry.verified_state(), &request)
                .unwrap_err()
                .kind(),
            ProjectBridgeErrorKind::ProjectRegistryRejected
        );
    }

    fn digest(c: char) -> ContentDigest {
        ContentDigest::from_sha256(c.to_string().repeat(64)).unwrap()
    }
    fn observation(branch: &str) -> RepositoryObservation {
        RepositoryObservation::new(
            r"C:\work\recovery",
            digest('1'),
            digest('2'),
            digest('3'),
            GitRefIdentity::new(branch, digest('4')).unwrap(),
        )
        .unwrap()
    }
    fn fixture(pending: RepositoryObservation) -> (FakeProjectRegistry, RecoveryRequest) {
        let mut registry = FakeProjectRegistry::new();
        let id = ProjectId::new("recovery-project").unwrap();
        registry
            .execute(RegistryCommand::register(
                CommandId::new("register").unwrap(),
                id.clone(),
                ProjectClass::UserProject,
                observation("refs/heads/main"),
            ))
            .unwrap();
        registry
            .execute(RegistryCommand::observe(
                CommandId::new("observe").unwrap(),
                id.clone(),
                registry.current_head(&id).unwrap(),
                pending.clone(),
            ))
            .unwrap();
        let head = registry.current_head(&id).unwrap();
        (
            registry,
            RecoveryRequest {
                project_id: id,
                revision: head.registry_revision(),
                receipt_digest: head.receipt_digest().clone(),
                pending_digest: pending.digest().clone(),
                restore_digest: None,
            },
        )
    }
    #[test]
    fn branch_recovery_applies_once_and_replays_original_receipt_after_later_change() {
        let pending = observation("refs/heads/feature");
        let (mut registry, request) = fixture(pending.clone());
        let command = prepare(registry.verified_state(), &request, &pending).unwrap();
        let receipt = registry.execute(command.clone()).unwrap();
        assert_eq!(receipt.outcome(), RegistryCommandOutcome::Applied);
        let checkpoint = registry.checkpoint().clone();
        assert_eq!(registry.execute(command).unwrap(), receipt);
        assert_eq!(registry.checkpoint(), &checkpoint);
        assert_eq!(
            replay(registry.verified_state(), &request).unwrap(),
            Some(&receipt)
        );
        registry
            .execute(RegistryCommand::observe(
                CommandId::new("later").unwrap(),
                request.project_id.clone(),
                registry.current_head(&request.project_id).unwrap(),
                observation("refs/heads/later"),
            ))
            .unwrap();
        assert_eq!(
            replay(registry.verified_state(), &request).unwrap(),
            Some(&receipt)
        );
        let current = registry
            .verified_state()
            .project(&request.project_id)
            .unwrap();
        assert_eq!(projection(current, &pending)["active"], false);
        assert_eq!(
            projection(current, &pending)["fresh_matches_pending"],
            false
        );
    }
    #[test]
    fn branch_recovery_rejects_wrong_project_stale_head_and_substituted_pending() {
        let pending = observation("refs/heads/feature");
        let (registry, request) = fixture(pending.clone());
        let mut bad = request.clone();
        bad.project_id = ProjectId::new("wrong-project").unwrap();
        assert!(prepare(registry.verified_state(), &bad, &pending).is_err());
        assert!(replay(registry.verified_state(), &bad).unwrap().is_none());
        let mut bad = request.clone();
        bad.revision -= 1;
        assert_eq!(
            prepare(registry.verified_state(), &bad, &pending)
                .unwrap_err()
                .kind(),
            ProjectBridgeErrorKind::ProjectRegistryConflict
        );
        let mut bad = request.clone();
        bad.receipt_digest = digest('a');
        assert!(prepare(registry.verified_state(), &bad, &pending).is_err());
        let mut bad = request.clone();
        bad.pending_digest = digest('b');
        assert!(prepare(registry.verified_state(), &bad, &pending).is_err());
        assert!(
            prepare(
                registry.verified_state(),
                &request,
                &observation("refs/heads/new-change")
            )
            .is_err()
        );
    }
    #[test]
    fn branch_recovery_rejects_moved_root_repository_and_worktree_changes() {
        for dimension in 0..4 {
            let pending = RepositoryObservation::new(
                if dimension == 0 {
                    r"C:\moved"
                } else {
                    r"C:\work\recovery"
                },
                digest(if dimension == 1 { 'a' } else { '1' }),
                digest(if dimension == 2 { 'b' } else { '2' }),
                digest(if dimension == 3 { 'c' } else { '3' }),
                GitRefIdentity::new("refs/heads/feature", digest('4')).unwrap(),
            )
            .unwrap();
            let (registry, request) = fixture(pending.clone());
            assert_eq!(
                prepare(registry.verified_state(), &request, &pending)
                    .unwrap_err()
                    .kind(),
                ProjectBridgeErrorKind::ProjectInactive
            );
        }
    }
    #[test]
    fn branch_recovery_rejects_suspension_and_never_refreshes_stale_request() {
        let pending = observation("refs/heads/feature");
        let (mut registry, request) = fixture(pending.clone());
        let prepared = prepare(registry.verified_state(), &request, &pending).unwrap();
        // Another reconciliation wins before execution: the domain rejects
        // the exact stale head even though our earlier preflight passed.
        let raced = registry
            .execute(RegistryCommand::reconcile(
                CommandId::new("race").unwrap(),
                request.project_id.clone(),
                registry.current_head(&request.project_id).unwrap(),
                pending.clone(),
                ReconciliationDecision::AcceptIdentityChange,
                digest('a'),
            ))
            .unwrap();
        assert_eq!(raced.outcome(), RegistryCommandOutcome::Applied);
        assert!(matches!(
            registry.execute(prepared).unwrap().outcome(),
            RegistryCommandOutcome::Denied(RegistryDenial::StaleHead)
        ));
        let (mut registry, request) = fixture(pending.clone());
        registry
            .execute(prepare(registry.verified_state(), &request, &pending).unwrap())
            .unwrap();
        registry
            .execute(RegistryCommand::suspend(
                CommandId::new("suspend").unwrap(),
                request.project_id.clone(),
                registry.current_head(&request.project_id).unwrap(),
                digest('a'),
            ))
            .unwrap();
        let head = registry.current_head(&request.project_id).unwrap();
        let request = RecoveryRequest {
            revision: head.registry_revision(),
            receipt_digest: head.receipt_digest().clone(),
            ..request
        };
        assert_eq!(
            prepare(registry.verified_state(), &request, &pending)
                .unwrap_err()
                .kind(),
            ProjectBridgeErrorKind::ProjectInactive
        );
    }
}
