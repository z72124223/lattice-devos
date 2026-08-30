//! Process-owned bridge from the Control project locator to live Registry authority.
//!
//! Control supplies only a loopback-local, non-authoritative locator. This
//! module independently observes the selected repository and is the only
//! place where that locator is translated into an existing durable Project
//! Registry command. It never accepts a path, Git executable, database target,
//! or Registry identity from MCP request bytes.

use std::env;
use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, SocketAddrV4, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(windows)]
use std::fs::OpenOptions;
#[cfg(windows)]
use std::os::windows::fs::OpenOptionsExt;
#[cfg(windows)]
use std::os::windows::io::AsRawHandle;

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
#[cfg(unix)]
use std::os::unix::io::AsRawFd;

#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_READ, FILE_SHARE_WRITE,
    GetFileInformationByHandle,
};

use lattice_cjson::{CanonicalValue, HashDomain, canonical_sha256};
use lattice_codex_adapter::SupervisedDuplexChild;
use lattice_contracts::{
    ContentDigest, GitRefIdentity, ProjectAuthorityHead, ProjectAuthorityReceipt, ProjectClass,
    ProjectId, ProjectLifecycle, StoreAuthorityHead,
};
use lattice_postgres_store::{
    MigrationTarget, PostgresProjectRegistry, PostgresProjectRegistryErrorKind,
};
use lattice_project_registry::{
    CommandId, RegistryCommand, RegistryCommandOutcome, RegistryDenial, RegistryProjectProjection,
    RepositoryObservation,
};
use lattice_task_ledger::task_submission_text_contains_secret;
use serde_json::{Map, Value};
use unicode_normalization::UnicodeNormalization;

use crate::delivery_ledger::{DeliveryDatabaseBinding, connect_fixed_runtime_client};
use crate::managed_file_identity::ManagedFileIdentity;

const DEFAULT_CONTROL_ORIGIN: &str = "http://127.0.0.1:4317";
const CONTROL_ORIGIN_ENV: &str = "LATTICE_CONTROL_ORIGIN";
const GIT_EXECUTABLE_ENV: &str = "LATTICE_DELIVERY_GIT_EXE";
const CATALOG_SCHEMA: &str = "lattice.control.project-catalog.v1";
const CATALOG_RECORD_KIND: &str = "CONTROL_LOCAL_CATALOG";
const CATALOG_AUTHORITY: &str = "NONE";
const MAX_CONTROL_RESPONSE_BYTES: u64 = 2 * 1024 * 1024;
const MAX_CONTROL_HEADER_BYTES: usize = 16 * 1024;
const MAX_CONTROL_PROJECTS: usize = 4_096;
const MAX_CONTROL_PROJECT_ID_BYTES: usize = 256;
const MAX_CONTROL_PROJECT_NAME_UTF16_UNITS: usize = 256;
const MAX_CONTROL_PROJECT_NAME_BYTES: usize = 1_024;
const MAX_PROJECT_NAME_CHARS: usize = 64;
const MAX_PROJECT_NAME_BYTES: usize = 256;
const MAX_GIT_OUTPUT_BYTES: usize = 128 * 1024;
const MAX_PROJECT_BRIDGE_GIT_BYTES: u64 = 512 * 1_024 * 1_024;
const IO_TIMEOUT_CAP: Duration = Duration::from_secs(5);
const MAX_REGISTRY_CURRENTNESS_RETRIES: usize = 1;

/// One request-side project selector. Neither variant can carry a path.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProjectSelector {
    project_id: Option<ProjectId>,
    project_name: Option<String>,
}

impl ProjectSelector {
    /// Validates an optional exact Control project ID or exact display name.
    ///
    /// Both absent is allowed and resolves only when exactly one eligible
    /// Control catalog row exists. Supplying both is rejected.
    pub(crate) fn new(
        project_id: Option<&str>,
        project_name: Option<&str>,
    ) -> ProjectBridgeResult<Self> {
        if project_id.is_some() && project_name.is_some() {
            return Err(bridge_error(ProjectBridgeErrorKind::InvalidSelector));
        }
        let project_id = project_id
            .map(|value| {
                if !safe_text(value, 64) || task_submission_text_contains_secret(value) {
                    return Err(bridge_error(ProjectBridgeErrorKind::InvalidSelector));
                }
                ProjectId::new(value.to_owned())
                    .map_err(|_| bridge_error(ProjectBridgeErrorKind::InvalidSelector))
            })
            .transpose()?;
        let project_name = project_name
            .map(|value| {
                if !safe_project_name(value) || value.nfc().ne(value.chars()) {
                    Err(bridge_error(ProjectBridgeErrorKind::InvalidSelector))
                } else {
                    Ok(value.to_owned())
                }
            })
            .transpose()?;
        Ok(Self {
            project_id,
            project_name,
        })
    }
}

/// Current live Project Registry authority obtained through the locator bridge.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedProjectAuthority {
    display_name: String,
    canonical_path: PathBuf,
    authority: ProjectAuthorityReceipt,
    current_head: ProjectAuthorityHead,
}

impl ResolvedProjectAuthority {
    #[must_use]
    pub(crate) fn display_name(&self) -> &str {
        &self.display_name
    }

    /// Verified canonical repository path from the twice-read Control locator.
    /// This remains a locator observation; Project Registry authority continues
    /// to own the durable project identity.
    #[must_use]
    pub(crate) fn canonical_path(&self) -> &Path {
        &self.canonical_path
    }

    #[must_use]
    pub(crate) const fn authority(&self) -> &ProjectAuthorityReceipt {
        &self.authority
    }

    #[must_use]
    pub(crate) const fn current_head(&self) -> &ProjectAuthorityHead {
        &self.current_head
    }
}

/// Closed failure taxonomy. Diagnostics never include a path, selector, Git
/// output, database error, or Control response body.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum ProjectBridgeErrorKind {
    InvalidSelector,
    ControlConfiguration,
    ControlUnavailable,
    ControlProtocol,
    ProjectNotFound,
    ProjectAmbiguous,
    ProjectNotRegistered,
    ProjectIdUnsupported,
    ProjectNameUnsupported,
    ProjectObservationUnavailable,
    ProjectFilesystemUnavailable,
    ProjectIdentityChanged,
    ProjectIdentityCollision,
    ProjectInactive,
    ProjectRegistryConflict,
    ProjectRegistryUnavailable,
    ProjectRegistryRejected,
    DeadlineExceeded,
}

impl ProjectBridgeErrorKind {
    #[must_use]
    pub(crate) const fn code(self) -> &'static str {
        match self {
            Self::InvalidSelector => "PROJECT_SELECTOR_INVALID",
            Self::ControlConfiguration => "CONTROL_PROJECT_ORIGIN_REJECTED",
            Self::ControlUnavailable => "CONTROL_PROJECT_CATALOG_UNAVAILABLE",
            Self::ControlProtocol => "CONTROL_PROJECT_CATALOG_INVALID",
            Self::ProjectNotFound => "REGISTERED_PROJECT_NOT_FOUND",
            Self::ProjectAmbiguous => "REGISTERED_PROJECT_AMBIGUOUS",
            Self::ProjectNotRegistered => "PROJECT_IS_NOT_REGISTERED",
            Self::ProjectIdUnsupported => "REGISTERED_PROJECT_ID_UNSUPPORTED",
            Self::ProjectNameUnsupported => "REGISTERED_PROJECT_NAME_UNSUPPORTED",
            Self::ProjectObservationUnavailable => "PROJECT_CATALOG_OBSERVATION_UNAVAILABLE",
            Self::ProjectFilesystemUnavailable => "PROJECT_LIVE_IDENTITY_UNAVAILABLE",
            Self::ProjectIdentityChanged => "PROJECT_IDENTITY_RECONCILIATION_REQUIRED",
            Self::ProjectIdentityCollision => "PROJECT_IDENTITY_COLLISION",
            Self::ProjectInactive => "PROJECT_REGISTRY_INACTIVE",
            Self::ProjectRegistryConflict => "PROJECT_REGISTRY_CURRENTNESS_CONFLICT",
            Self::ProjectRegistryUnavailable => "PROJECT_REGISTRY_UNAVAILABLE",
            Self::ProjectRegistryRejected => "PROJECT_REGISTRY_REJECTED",
            Self::DeadlineExceeded => "PROJECT_BRIDGE_DEADLINE_EXCEEDED",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ProjectBridgeError {
    kind: ProjectBridgeErrorKind,
}

impl ProjectBridgeError {
    #[cfg(test)]
    #[must_use]
    pub(crate) const fn kind(self) -> ProjectBridgeErrorKind {
        self.kind
    }

    #[must_use]
    pub(crate) const fn code(self) -> &'static str {
        self.kind.code()
    }
}

impl fmt::Display for ProjectBridgeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl Error for ProjectBridgeError {}

pub(crate) type ProjectBridgeResult<T> = Result<T, ProjectBridgeError>;

/// Resolves one Control locator, performs a fresh physical repository
/// observation, and obtains current authority from the existing `PostgreSQL`
/// Project Registry.
///
/// The origin, Git executable, database binding, password, and daemon
/// authority are all process-owned inputs. The selector contains no path.
pub(crate) fn resolve_project_authority(
    database: &DeliveryDatabaseBinding,
    password: &str,
    deadline: Instant,
    store_authority: &StoreAuthorityHead,
    selector: &ProjectSelector,
) -> ProjectBridgeResult<ResolvedProjectAuthority> {
    ensure_before(deadline)?;
    let origin_value = match env::var_os(CONTROL_ORIGIN_ENV) {
        None => DEFAULT_CONTROL_ORIGIN.to_owned(),
        Some(value) => value
            .into_string()
            .map_err(|_| bridge_error(ProjectBridgeErrorKind::ControlConfiguration))?,
    };
    let origin = parse_control_origin(&origin_value)
        .ok_or_else(|| bridge_error(ProjectBridgeErrorKind::ControlConfiguration))?;
    let git_executable = configured_git_executable()?;

    let mut effective_project_id = None;
    for currentness_retry in 0..=MAX_REGISTRY_CURRENTNESS_RETRIES {
        let state = control_get_json(origin, "/api/state", deadline)?;
        let locators = parse_catalog_state(&state)?;
        let selected = select_catalog_project(&locators, selector)?;
        retain_effective_project_id(&mut effective_project_id, &selected.id)?;
        let detail_path = format!("/api/projects/{}", selected.id);
        let first_detail_value = control_get_json(origin, &detail_path, deadline)?;
        let first_detail = parse_catalog_detail(&first_detail_value, Some(&selected))?;
        let observation =
            inspect_repository(&first_detail.canonical_path, &git_executable, deadline)?;

        // Re-read both the complete eligible catalog selection surface and the selected
        // detail after the physical observation. This prevents a name/no-selector
        // match from becoming ambiguous, disappearing, or changing while Git is
        // inspected, even when the originally selected detail itself is unchanged.
        let replay_state = control_get_json(origin, "/api/state", deadline)?;
        verify_catalog_replay(&state, &replay_state, selector, &selected)?;
        let replay_value = control_get_json(origin, &detail_path, deadline)?;
        if replay_value != first_detail_value {
            return Err(bridge_error(ProjectBridgeErrorKind::ProjectIdentityChanged));
        }
        let replay = parse_catalog_detail(&replay_value, Some(&selected))?;
        if replay != first_detail {
            return Err(bridge_error(ProjectBridgeErrorKind::ControlProtocol));
        }

        match resolve_registry_authority(
            database,
            password,
            deadline,
            store_authority,
            &replay,
            &observation,
        ) {
            Err(error)
                if currentness_retry < MAX_REGISTRY_CURRENTNESS_RETRIES
                    && error.kind == ProjectBridgeErrorKind::ProjectRegistryConflict =>
            {
                // The bounded outer loop performs the one allowed currentness retry.
            }
            result => return result,
        }
    }
    Err(bridge_error(
        ProjectBridgeErrorKind::ProjectRegistryConflict,
    ))
}

fn verify_catalog_replay(
    first_state: &Value,
    replay_state: &Value,
    selector: &ProjectSelector,
    expected: &CatalogLocator,
) -> ProjectBridgeResult<CatalogLocator> {
    let first_locators = parse_catalog_state(first_state)?;
    let replay_locators = parse_catalog_state(replay_state)?;
    if !same_catalog_surface(&first_locators, &replay_locators) {
        return Err(bridge_error(ProjectBridgeErrorKind::ProjectIdentityChanged));
    }
    let replay = select_catalog_project(&replay_locators, selector)
        .map_err(|_| bridge_error(ProjectBridgeErrorKind::ProjectIdentityChanged))?;
    if &replay != expected {
        return Err(bridge_error(ProjectBridgeErrorKind::ProjectIdentityChanged));
    }
    Ok(replay)
}

fn same_catalog_surface(first: &[CatalogLocator], replay: &[CatalogLocator]) -> bool {
    let mut first = first
        .iter()
        .filter(|project| project.eligible)
        .cloned()
        .collect::<Vec<_>>();
    let mut replay = replay
        .iter()
        .filter(|project| project.eligible)
        .cloned()
        .collect::<Vec<_>>();
    let sort = |projects: &mut Vec<CatalogLocator>| {
        projects.sort_by(|left, right| {
            (
                &left.id,
                &left.name,
                &left.canonical_path,
                left.eligible,
                left.id_supported,
                left.name_supported,
            )
                .cmp(&(
                    &right.id,
                    &right.name,
                    &right.canonical_path,
                    right.eligible,
                    right.id_supported,
                    right.name_supported,
                ))
        });
    };
    sort(&mut first);
    sort(&mut replay);
    first == replay
}

fn retain_effective_project_id(
    effective_project_id: &mut Option<String>,
    selected_project_id: &str,
) -> ProjectBridgeResult<()> {
    if effective_project_id
        .as_ref()
        .is_some_and(|expected| expected != selected_project_id)
    {
        return Err(bridge_error(ProjectBridgeErrorKind::ProjectIdentityChanged));
    }
    effective_project_id.get_or_insert_with(|| selected_project_id.to_owned());
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ControlOrigin {
    port: u16,
}

fn parse_control_origin(value: &str) -> Option<ControlOrigin> {
    let port = value.strip_prefix("http://127.0.0.1:")?;
    if port.is_empty() || port.starts_with('0') || !port.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let port = port.parse::<u16>().ok()?;
    (port != 0).then_some(ControlOrigin { port })
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CatalogLocator {
    id: String,
    id_supported: bool,
    name: String,
    name_supported: bool,
    canonical_path: PathBuf,
    eligible: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CatalogProject {
    id: ProjectId,
    name: String,
    canonical_path: PathBuf,
}

fn parse_catalog_state(value: &Value) -> ProjectBridgeResult<Vec<CatalogLocator>> {
    let object = value
        .as_object()
        .ok_or_else(|| bridge_error(ProjectBridgeErrorKind::ControlProtocol))?;
    let projects = object
        .get("projects")
        .and_then(Value::as_array)
        .ok_or_else(|| bridge_error(ProjectBridgeErrorKind::ControlProtocol))?;
    if projects.len() > MAX_CONTROL_PROJECTS {
        return Err(bridge_error(ProjectBridgeErrorKind::ControlProtocol));
    }
    projects.iter().map(parse_catalog_locator).collect()
}

fn parse_catalog_locator(value: &Value) -> ProjectBridgeResult<CatalogLocator> {
    let object = value
        .as_object()
        .ok_or_else(|| bridge_error(ProjectBridgeErrorKind::ControlProtocol))?;
    let id_text = required_safe_string(object, "id", MAX_CONTROL_PROJECT_ID_BYTES)?;
    let id_supported = ProjectId::new(id_text.to_owned()).is_ok()
        && !task_submission_text_contains_secret(id_text);
    let name = required_safe_string(object, "name", MAX_CONTROL_PROJECT_NAME_BYTES)?;
    if name.encode_utf16().count() > MAX_CONTROL_PROJECT_NAME_UTF16_UNITS {
        return Err(bridge_error(ProjectBridgeErrorKind::ControlProtocol));
    }
    let name_supported = safe_project_name(name) && name.nfc().eq(name.chars());
    let common_boundary = object.get("registry_authority").and_then(Value::as_str)
        == Some(CATALOG_AUTHORITY)
        && object
            .get("registry_project_id")
            .is_some_and(Value::is_null)
        && object.get("control_project_id").and_then(Value::as_str) == Some(id_text);
    let eligible = common_boundary
        && object.get("schema_version").and_then(Value::as_str) == Some(CATALOG_SCHEMA)
        && object.get("record_kind").and_then(Value::as_str) == Some(CATALOG_RECORD_KIND);
    let legacy = common_boundary
        && object.get("schema_version").is_some_and(Value::is_null)
        && object.get("record_kind").and_then(Value::as_str) == Some("LEGACY_CONTROL_PROJECT");
    if !eligible && !legacy {
        return Err(bridge_error(ProjectBridgeErrorKind::ControlProtocol));
    }
    let canonical_path = if eligible {
        validated_catalog_path(required_safe_string(
            object,
            "canonical_path",
            lattice_project_registry::MAX_CANONICAL_ROOT_BYTES,
        )?)?
    } else {
        PathBuf::new()
    };
    Ok(CatalogLocator {
        id: id_text.to_owned(),
        id_supported,
        name: name.to_owned(),
        name_supported,
        canonical_path,
        eligible,
    })
}

fn select_catalog_project(
    locators: &[CatalogLocator],
    selector: &ProjectSelector,
) -> ProjectBridgeResult<CatalogLocator> {
    if let Some(project_id) = selector.project_id.as_ref() {
        let mut matches = locators
            .iter()
            .filter(|project| project.id == project_id.as_str());
        let selected = matches
            .next()
            .ok_or_else(|| bridge_error(ProjectBridgeErrorKind::ProjectNotFound))?;
        if matches.next().is_some() {
            return Err(bridge_error(ProjectBridgeErrorKind::ControlProtocol));
        }
        if !selected.eligible {
            return Err(bridge_error(ProjectBridgeErrorKind::ProjectNotRegistered));
        }
        if !selected.id_supported {
            return Err(bridge_error(ProjectBridgeErrorKind::ProjectIdUnsupported));
        }
        if !selected.name_supported {
            return Err(bridge_error(ProjectBridgeErrorKind::ProjectNameUnsupported));
        }
        return Ok(selected.clone());
    }

    let eligible = locators.iter().filter(|project| project.eligible);
    let matches = if let Some(project_name) = selector.project_name.as_ref() {
        eligible
            .filter(|project| project.name.nfc().eq(project_name.chars()))
            .collect::<Vec<_>>()
    } else {
        eligible.collect::<Vec<_>>()
    };
    match matches.as_slice() {
        [selected] if !selected.id_supported => {
            Err(bridge_error(ProjectBridgeErrorKind::ProjectIdUnsupported))
        }
        [selected] if selected.name_supported => Ok((*selected).clone()),
        [_] => Err(bridge_error(ProjectBridgeErrorKind::ProjectNameUnsupported)),
        [] => Err(bridge_error(ProjectBridgeErrorKind::ProjectNotFound)),
        _ => Err(bridge_error(ProjectBridgeErrorKind::ProjectAmbiguous)),
    }
}

fn parse_catalog_detail(
    value: &Value,
    expected: Option<&CatalogLocator>,
) -> ProjectBridgeResult<CatalogProject> {
    let object = value
        .as_object()
        .ok_or_else(|| bridge_error(ProjectBridgeErrorKind::ControlProtocol))?;
    let locator = parse_catalog_locator(value)?;
    if !locator.eligible {
        return Err(bridge_error(ProjectBridgeErrorKind::ProjectNotRegistered));
    }
    if !locator.id_supported {
        return Err(bridge_error(ProjectBridgeErrorKind::ProjectIdUnsupported));
    }
    if !locator.name_supported {
        return Err(bridge_error(ProjectBridgeErrorKind::ProjectNameUnsupported));
    }
    if expected.is_some_and(|expected| {
        locator.id != expected.id
            || locator.name != expected.name
            || locator.canonical_path != expected.canonical_path
    }) {
        return Err(bridge_error(ProjectBridgeErrorKind::ProjectIdentityChanged));
    }
    if !object
        .get("last_refresh_failure")
        .is_some_and(Value::is_null)
        || object.get("repo_root").and_then(Value::as_str) != locator.canonical_path.to_str()
    {
        return Err(bridge_error(
            ProjectBridgeErrorKind::ProjectObservationUnavailable,
        ));
    }
    for timestamp in ["created_at", "updated_at", "registered_at", "refreshed_at"] {
        required_safe_string(object, timestamp, 128)?;
    }
    let git = required_object(object, "git_observation")?;
    let git_complete = git.get("status").and_then(Value::as_str) == Some("complete")
        && git.get("is_repository").and_then(Value::as_bool) == Some(true)
        && git.get("detached").and_then(Value::as_bool) == Some(false)
        && git
            .get("branch")
            .and_then(Value::as_str)
            .is_some_and(|branch| safe_text(branch, 1_024))
        && git
            .get("failures")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty);
    let rules = required_object(object, "rule_index")?;
    let rules_complete = rules.get("status").and_then(Value::as_str) == Some("complete")
        && rules
            .get("failures")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty);
    if !git_complete || !rules_complete {
        return Err(bridge_error(
            ProjectBridgeErrorKind::ProjectObservationUnavailable,
        ));
    }
    let id = ProjectId::new(locator.id)
        .map_err(|_| bridge_error(ProjectBridgeErrorKind::ProjectIdUnsupported))?;
    Ok(CatalogProject {
        id,
        name: locator.name,
        canonical_path: locator.canonical_path,
    })
}

fn required_object<'a>(
    object: &'a Map<String, Value>,
    field: &str,
) -> ProjectBridgeResult<&'a Map<String, Value>> {
    object
        .get(field)
        .and_then(Value::as_object)
        .ok_or_else(|| bridge_error(ProjectBridgeErrorKind::ControlProtocol))
}

fn required_safe_string<'a>(
    object: &'a Map<String, Value>,
    field: &str,
    max_bytes: usize,
) -> ProjectBridgeResult<&'a str> {
    object
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| safe_text(value, max_bytes))
        .ok_or_else(|| bridge_error(ProjectBridgeErrorKind::ControlProtocol))
}

fn safe_text(value: &str, max_bytes: usize) -> bool {
    !value.is_empty()
        && value.trim() == value
        && value.len() <= max_bytes
        && !value.chars().any(char::is_control)
}

fn safe_project_name(value: &str) -> bool {
    safe_text(value, MAX_PROJECT_NAME_BYTES)
        && value.chars().count() <= MAX_PROJECT_NAME_CHARS
        && !task_submission_text_contains_secret(value)
}

fn validated_catalog_path(value: &str) -> ProjectBridgeResult<PathBuf> {
    if value.nfc().ne(value.chars()) {
        return Err(bridge_error(ProjectBridgeErrorKind::ControlProtocol));
    }
    let path = PathBuf::from(value);
    if !path.is_absolute() {
        return Err(bridge_error(ProjectBridgeErrorKind::ControlProtocol));
    }
    Ok(path)
}

fn control_get_json(
    origin: ControlOrigin,
    path: &str,
    deadline: Instant,
) -> ProjectBridgeResult<Value> {
    if !path.starts_with('/')
        || path
            .bytes()
            .any(|byte| byte.is_ascii_control() || byte == b' ')
    {
        return Err(bridge_error(ProjectBridgeErrorKind::ControlProtocol));
    }
    let timeout = remaining(deadline)?.min(IO_TIMEOUT_CAP);
    let address = SocketAddrV4::new(Ipv4Addr::LOCALHOST, origin.port);
    let mut stream = TcpStream::connect_timeout(&address.into(), timeout)
        .map_err(|_| bridge_error(ProjectBridgeErrorKind::ControlUnavailable))?;
    let io_timeout = remaining(deadline)?.min(IO_TIMEOUT_CAP);
    stream
        .set_read_timeout(Some(io_timeout))
        .and_then(|()| stream.set_write_timeout(Some(io_timeout)))
        .map_err(|_| bridge_error(ProjectBridgeErrorKind::ControlUnavailable))?;
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAccept: application/json\r\nConnection: close\r\n\r\n",
        origin.port
    );
    stream
        .write_all(request.as_bytes())
        .and_then(|()| stream.flush())
        .map_err(|_| bridge_error(ProjectBridgeErrorKind::ControlUnavailable))?;
    let mut response = Vec::new();
    stream
        .take(MAX_CONTROL_RESPONSE_BYTES + 1)
        .read_to_end(&mut response)
        .map_err(|_| bridge_error(ProjectBridgeErrorKind::ControlUnavailable))?;
    ensure_before(deadline)?;
    if response.len() as u64 > MAX_CONTROL_RESPONSE_BYTES {
        return Err(bridge_error(ProjectBridgeErrorKind::ControlProtocol));
    }
    parse_http_json(&response)
}

fn parse_http_json(response: &[u8]) -> ProjectBridgeResult<Value> {
    let separator = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or_else(|| bridge_error(ProjectBridgeErrorKind::ControlProtocol))?;
    if separator > MAX_CONTROL_HEADER_BYTES {
        return Err(bridge_error(ProjectBridgeErrorKind::ControlProtocol));
    }
    let headers = std::str::from_utf8(&response[..separator])
        .map_err(|_| bridge_error(ProjectBridgeErrorKind::ControlProtocol))?;
    let mut lines = headers.split("\r\n");
    let status = lines
        .next()
        .ok_or_else(|| bridge_error(ProjectBridgeErrorKind::ControlProtocol))?;
    let mut status_parts = status.split_ascii_whitespace();
    if status_parts.next() != Some("HTTP/1.1") || status_parts.next() != Some("200") {
        return Err(bridge_error(ProjectBridgeErrorKind::ControlUnavailable));
    }
    let mut content_length = None;
    let mut content_type = None;
    for line in lines {
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| bridge_error(ProjectBridgeErrorKind::ControlProtocol))?;
        let name = name.trim().to_ascii_lowercase();
        let value = value.trim();
        match name.as_str() {
            "content-length" => {
                if content_length.is_some() {
                    return Err(bridge_error(ProjectBridgeErrorKind::ControlProtocol));
                }
                content_length = value.parse::<usize>().ok();
                if content_length.is_none() {
                    return Err(bridge_error(ProjectBridgeErrorKind::ControlProtocol));
                }
            }
            "content-type" => {
                if content_type.replace(value.to_ascii_lowercase()).is_some() {
                    return Err(bridge_error(ProjectBridgeErrorKind::ControlProtocol));
                }
            }
            "transfer-encoding" => {
                return Err(bridge_error(ProjectBridgeErrorKind::ControlProtocol));
            }
            _ => {}
        }
    }
    let body = &response[(separator + 4)..];
    if content_length != Some(body.len())
        || !content_type.is_some_and(|value| value.starts_with("application/json"))
    {
        return Err(bridge_error(ProjectBridgeErrorKind::ControlProtocol));
    }
    serde_json::from_slice(body).map_err(|_| bridge_error(ProjectBridgeErrorKind::ControlProtocol))
}

fn configured_git_executable() -> ProjectBridgeResult<PathBuf> {
    let declared = env::var_os(GIT_EXECUTABLE_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| bridge_error(ProjectBridgeErrorKind::ProjectFilesystemUnavailable))?;
    let metadata = fs::symlink_metadata(&declared)
        .map_err(|_| bridge_error(ProjectBridgeErrorKind::ProjectFilesystemUnavailable))?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(bridge_error(
            ProjectBridgeErrorKind::ProjectFilesystemUnavailable,
        ));
    }
    fs::canonicalize(declared)
        .map_err(|_| bridge_error(ProjectBridgeErrorKind::ProjectFilesystemUnavailable))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PhysicalIdentity {
    device: u64,
    file: u64,
}

#[derive(Debug)]
struct CapturedDirectory {
    canonical_path: PathBuf,
    identity: PhysicalIdentity,
    _handle: File,
}

#[derive(Debug)]
struct RepositoryProbe {
    root: CapturedDirectory,
    git_directory: CapturedDirectory,
    common_directory: CapturedDirectory,
    primary_ref: String,
}

fn inspect_repository(
    catalog_path: &Path,
    git_executable: &Path,
    deadline: Instant,
) -> ProjectBridgeResult<RepositoryObservation> {
    let git_identity = ManagedFileIdentity::capture(git_executable, MAX_PROJECT_BRIDGE_GIT_BYTES)
        .map_err(|_| bridge_error(ProjectBridgeErrorKind::ProjectIdentityChanged))?;
    let first = repository_probe(catalog_path, git_executable, &git_identity, deadline)?;
    let second = repository_probe(catalog_path, git_executable, &git_identity, deadline)?;
    if !same_probe(&first, &second) {
        return Err(bridge_error(ProjectBridgeErrorKind::ProjectIdentityChanged));
    }
    let canonical_root = first
        .root
        .canonical_path
        .to_str()
        .filter(|value| safe_text(value, lattice_project_registry::MAX_CANONICAL_ROOT_BYTES))
        .filter(|value| value.nfc().eq(value.chars()))
        .ok_or_else(|| bridge_error(ProjectBridgeErrorKind::ProjectFilesystemUnavailable))?;
    let root_digest = physical_digest("lattice.project-bridge.root-identity", first.root.identity)?;
    let repository_digest = physical_digest(
        "lattice.project-bridge.repository-identity",
        first.common_directory.identity,
    )?;
    let file_digest = paired_physical_digest(
        "lattice.project-bridge.worktree-identity",
        first.root.identity,
        first.git_directory.identity,
    )?;
    let primary_ref_digest =
        ref_storage_digest(first.common_directory.identity, &first.primary_ref)?;
    let primary_branch = GitRefIdentity::new(first.primary_ref, primary_ref_digest)
        .map_err(|_| bridge_error(ProjectBridgeErrorKind::ProjectFilesystemUnavailable))?;
    RepositoryObservation::new(
        canonical_root.to_owned(),
        root_digest,
        repository_digest,
        file_digest,
        primary_branch,
    )
    .map_err(|_| bridge_error(ProjectBridgeErrorKind::ProjectFilesystemUnavailable))
}

fn repository_probe(
    catalog_path: &Path,
    git_executable: &Path,
    git_identity: &ManagedFileIdentity,
    deadline: Instant,
) -> ProjectBridgeResult<RepositoryProbe> {
    ensure_before(deadline)?;
    let root = capture_directory(catalog_path)?;
    let top_level = git_stdout(
        git_executable,
        git_identity,
        &root,
        ["rev-parse", "--show-toplevel"],
        deadline,
    )?;
    let observed_root = capture_directory(Path::new(&top_level))?;
    if observed_root.identity != root.identity {
        return Err(bridge_error(ProjectBridgeErrorKind::ProjectIdentityChanged));
    }
    let git_directory_path = git_stdout(
        git_executable,
        git_identity,
        &root,
        ["rev-parse", "--path-format=absolute", "--git-dir"],
        deadline,
    )?;
    let common_directory_path = git_stdout(
        git_executable,
        git_identity,
        &root,
        ["rev-parse", "--path-format=absolute", "--git-common-dir"],
        deadline,
    )?;
    let primary_ref = git_stdout(
        git_executable,
        git_identity,
        &root,
        ["symbolic-ref", "--quiet", "HEAD"],
        deadline,
    )?;
    // Construction here validates the fully-qualified local branch before any
    // value can enter a Registry observation.
    GitRefIdentity::new(primary_ref.clone(), zero_digest()?)
        .map_err(|_| bridge_error(ProjectBridgeErrorKind::ProjectFilesystemUnavailable))?;
    let git_directory = capture_directory(Path::new(&git_directory_path))?;
    let common_directory = capture_directory(Path::new(&common_directory_path))?;
    ensure_before(deadline)?;
    Ok(RepositoryProbe {
        root,
        git_directory,
        common_directory,
        primary_ref,
    })
}

fn same_probe(first: &RepositoryProbe, second: &RepositoryProbe) -> bool {
    first.root.canonical_path == second.root.canonical_path
        && first.root.identity == second.root.identity
        && first.git_directory.canonical_path == second.git_directory.canonical_path
        && first.git_directory.identity == second.git_directory.identity
        && first.common_directory.canonical_path == second.common_directory.canonical_path
        && first.common_directory.identity == second.common_directory.identity
        && first.primary_ref == second.primary_ref
}

fn capture_directory(path: &Path) -> ProjectBridgeResult<CapturedDirectory> {
    let initial = fs::symlink_metadata(path)
        .map_err(|_| bridge_error(ProjectBridgeErrorKind::ProjectFilesystemUnavailable))?;
    if !initial.file_type().is_dir() || initial.file_type().is_symlink() {
        return Err(bridge_error(
            ProjectBridgeErrorKind::ProjectFilesystemUnavailable,
        ));
    }
    let canonical_path = fs::canonicalize(path)
        .map_err(|_| bridge_error(ProjectBridgeErrorKind::ProjectFilesystemUnavailable))?;
    let final_metadata = fs::symlink_metadata(&canonical_path)
        .map_err(|_| bridge_error(ProjectBridgeErrorKind::ProjectFilesystemUnavailable))?;
    if !final_metadata.file_type().is_dir() || final_metadata.file_type().is_symlink() {
        return Err(bridge_error(
            ProjectBridgeErrorKind::ProjectFilesystemUnavailable,
        ));
    }
    let handle = open_directory(&canonical_path)?;
    let identity = physical_identity(&handle)?;
    let replay = open_directory(&canonical_path)?;
    if physical_identity(&replay)? != identity {
        return Err(bridge_error(ProjectBridgeErrorKind::ProjectIdentityChanged));
    }
    Ok(CapturedDirectory {
        canonical_path,
        identity,
        _handle: handle,
    })
}

#[cfg(windows)]
fn open_directory(path: &Path) -> ProjectBridgeResult<File> {
    let mut options = OpenOptions::new();
    options
        .read(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS);
    options
        .open(path)
        .map_err(|_| bridge_error(ProjectBridgeErrorKind::ProjectFilesystemUnavailable))
}

#[cfg(not(windows))]
fn open_directory(path: &Path) -> ProjectBridgeResult<File> {
    File::open(path).map_err(|_| bridge_error(ProjectBridgeErrorKind::ProjectFilesystemUnavailable))
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn physical_identity(file: &File) -> ProjectBridgeResult<PhysicalIdentity> {
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    if unsafe { GetFileInformationByHandle(file.as_raw_handle().cast(), &raw mut information) } == 0
    {
        return Err(bridge_error(
            ProjectBridgeErrorKind::ProjectFilesystemUnavailable,
        ));
    }
    Ok(PhysicalIdentity {
        device: u64::from(information.dwVolumeSerialNumber),
        file: (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow),
    })
}

#[cfg(unix)]
fn physical_identity(file: &File) -> ProjectBridgeResult<PhysicalIdentity> {
    let metadata = file
        .metadata()
        .map_err(|_| bridge_error(ProjectBridgeErrorKind::ProjectFilesystemUnavailable))?;
    Ok(PhysicalIdentity {
        device: metadata.dev(),
        file: metadata.ino(),
    })
}

#[cfg(not(any(windows, unix)))]
fn physical_identity(_file: &File) -> ProjectBridgeResult<PhysicalIdentity> {
    Err(bridge_error(
        ProjectBridgeErrorKind::ProjectFilesystemUnavailable,
    ))
}

fn git_stdout<I, S>(
    executable: &Path,
    executable_identity: &ManagedFileIdentity,
    repository_root: &CapturedDirectory,
    arguments: I,
    deadline: Instant,
) -> ProjectBridgeResult<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    git_stdout_from_captured_with_hooks(
        executable,
        executable_identity,
        repository_root,
        arguments,
        deadline,
        |_| {},
        || {},
    )
}

fn git_stdout_from_captured_with_hooks<I, S, Configure, PostSeal>(
    executable: &Path,
    executable_identity: &ManagedFileIdentity,
    repository_root: &CapturedDirectory,
    arguments: I,
    deadline: Instant,
    pre_configuration_hook: Configure,
    post_seal_hook: PostSeal,
) -> ProjectBridgeResult<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
    Configure: FnOnce(&mut Command),
    PostSeal: FnOnce(),
{
    let arguments = project_git_arguments(repository_root, arguments)?;
    let child_root = captured_directory_child_path(repository_root)?;
    git_stdout_with_hooks(
        executable,
        executable_identity,
        &child_root,
        arguments,
        deadline,
        pre_configuration_hook,
        post_seal_hook,
        Some(repository_root),
    )
}

fn project_git_arguments<I, S>(
    repository_root: &CapturedDirectory,
    arguments: I,
) -> ProjectBridgeResult<Vec<OsString>>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    verify_captured_directory_current(repository_root)?;
    let safe_directory = git_safe_directory_value(&repository_root.canonical_path)?;
    let mut configured = vec![
        OsString::from("-c"),
        OsString::from(format!("safe.directory={safe_directory}")),
    ];
    configured.extend(
        arguments
            .into_iter()
            .map(|argument| argument.as_ref().to_os_string()),
    );
    Ok(configured)
}

fn verify_captured_directory_current(
    repository_root: &CapturedDirectory,
) -> ProjectBridgeResult<()> {
    let replay = capture_directory(&repository_root.canonical_path)
        .map_err(|_| bridge_error(ProjectBridgeErrorKind::ProjectIdentityChanged))?;
    if replay.canonical_path != repository_root.canonical_path
        || replay.identity != repository_root.identity
    {
        return Err(bridge_error(ProjectBridgeErrorKind::ProjectIdentityChanged));
    }
    Ok(())
}

#[cfg(windows)]
fn captured_directory_child_path(
    repository_root: &CapturedDirectory,
) -> ProjectBridgeResult<PathBuf> {
    verify_captured_directory_current(repository_root)?;
    windows_child_directory_path(&repository_root.canonical_path)
}

#[cfg(windows)]
fn windows_child_directory_path(path: &Path) -> ProjectBridgeResult<PathBuf> {
    const VERBATIM_UNC_PREFIX: &str = r"\\?\UNC\";

    if !path.is_absolute() {
        return Err(bridge_error(
            ProjectBridgeErrorKind::ProjectFilesystemUnavailable,
        ));
    }
    let value = path
        .to_str()
        .ok_or_else(|| bridge_error(ProjectBridgeErrorKind::ProjectFilesystemUnavailable))?;
    let child_path = if value
        .get(..VERBATIM_UNC_PREFIX.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(VERBATIM_UNC_PREFIX))
    {
        PathBuf::from(format!(r"\\{}", &value[VERBATIM_UNC_PREFIX.len()..]))
    } else {
        path.to_path_buf()
    };
    if !child_path.is_absolute() {
        return Err(bridge_error(
            ProjectBridgeErrorKind::ProjectFilesystemUnavailable,
        ));
    }
    Ok(child_path)
}

#[cfg(unix)]
fn captured_directory_child_path(
    repository_root: &CapturedDirectory,
) -> ProjectBridgeResult<PathBuf> {
    #[cfg(target_os = "linux")]
    let descriptor_root = Path::new("/proc/self/fd");
    #[cfg(not(target_os = "linux"))]
    let descriptor_root = Path::new("/dev/fd");
    let child_root = descriptor_root.join(repository_root._handle.as_raw_fd().to_string());
    if !fs::metadata(&child_root).is_ok_and(|metadata| metadata.is_dir()) {
        return Err(bridge_error(
            ProjectBridgeErrorKind::ProjectFilesystemUnavailable,
        ));
    }
    Ok(child_root)
}

#[cfg(not(any(windows, unix)))]
fn captured_directory_child_path(
    _repository_root: &CapturedDirectory,
) -> ProjectBridgeResult<PathBuf> {
    Err(bridge_error(
        ProjectBridgeErrorKind::ProjectFilesystemUnavailable,
    ))
}

#[cfg(windows)]
fn git_safe_directory_value(path: &Path) -> ProjectBridgeResult<String> {
    const VERBATIM_PREFIX: &str = r"\\?\";
    const VERBATIM_UNC_PREFIX: &str = r"\\?\UNC\";

    if !path.is_absolute() {
        return Err(bridge_error(
            ProjectBridgeErrorKind::ProjectFilesystemUnavailable,
        ));
    }
    let value = path
        .to_str()
        .ok_or_else(|| bridge_error(ProjectBridgeErrorKind::ProjectFilesystemUnavailable))?;
    let native = if value
        .get(..VERBATIM_UNC_PREFIX.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(VERBATIM_UNC_PREFIX))
    {
        format!("//{}", &value[VERBATIM_UNC_PREFIX.len()..])
    } else if let Some(suffix) = value.strip_prefix(VERBATIM_PREFIX) {
        suffix.to_owned()
    } else {
        value.to_owned()
    };
    validate_git_safe_directory_value(native.replace('\\', "/"))
}

#[cfg(not(windows))]
fn git_safe_directory_value(path: &Path) -> ProjectBridgeResult<String> {
    if !path.is_absolute() {
        return Err(bridge_error(
            ProjectBridgeErrorKind::ProjectFilesystemUnavailable,
        ));
    }
    let value = path
        .to_str()
        .ok_or_else(|| bridge_error(ProjectBridgeErrorKind::ProjectFilesystemUnavailable))?;
    validate_git_safe_directory_value(value.to_owned())
}

fn validate_git_safe_directory_value(value: String) -> ProjectBridgeResult<String> {
    if !safe_text(&value, lattice_project_registry::MAX_CANONICAL_ROOT_BYTES)
        || value.nfc().ne(value.chars())
        || value.contains('*')
    {
        return Err(bridge_error(
            ProjectBridgeErrorKind::ProjectFilesystemUnavailable,
        ));
    }
    Ok(value)
}

fn git_stdout_with_hooks<I, S, Configure, PostSeal>(
    executable: &Path,
    executable_identity: &ManagedFileIdentity,
    repository_root: &Path,
    arguments: I,
    deadline: Instant,
    pre_configuration_hook: Configure,
    post_seal_hook: PostSeal,
    captured_repository_root: Option<&CapturedDirectory>,
) -> ProjectBridgeResult<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
    Configure: FnOnce(&mut Command),
    PostSeal: FnOnce(),
{
    ensure_before(deadline)?;
    let _executable_seal = executable_identity
        .seal()
        .map_err(|_| bridge_error(ProjectBridgeErrorKind::ProjectIdentityChanged))?;
    let mut command = Command::new(executable);
    command.current_dir(repository_root).args(arguments);
    pre_configuration_hook(&mut command);
    configure_project_git_environment(&mut command);
    post_seal_hook();
    if let Some(repository_root) = captured_repository_root {
        verify_captured_directory_current(repository_root)?;
    }
    executable_identity
        .verify()
        .map_err(|_| bridge_error(ProjectBridgeErrorKind::ProjectIdentityChanged))?;
    ensure_before(deadline)?;
    let mut child = SupervisedDuplexChild::spawn_with_stderr_cleared(&mut command)
        .map_err(|_| bridge_error(ProjectBridgeErrorKind::ProjectFilesystemUnavailable))?;
    let mut stdout_reader = None;
    let mut stderr_reader = None;
    let operation_result = (|| -> ProjectBridgeResult<ExitStatus> {
        child.take_stdin();
        let stdout = child
            .take_stdout()
            .ok_or_else(|| bridge_error(ProjectBridgeErrorKind::ProjectFilesystemUnavailable))?;
        let stderr = child
            .take_stderr()
            .ok_or_else(|| bridge_error(ProjectBridgeErrorKind::ProjectFilesystemUnavailable))?;
        stdout_reader = Some(spawn_bounded_git_reader(stdout));
        stderr_reader = Some(spawn_bounded_git_reader(stderr));
        wait_for_supervised_child(&mut child, deadline)
    })();
    let cleanup = child
        .terminate_and_reap()
        .map_err(|_| bridge_error(ProjectBridgeErrorKind::ProjectFilesystemUnavailable));
    drop(child);
    let stdout = join_bounded_git_reader(stdout_reader)?;
    let stderr = join_bounded_git_reader(stderr_reader)?;
    cleanup?;
    let status = operation_result?;
    if !status.success() || !stderr.is_empty() {
        return Err(bridge_error(
            ProjectBridgeErrorKind::ProjectFilesystemUnavailable,
        ));
    }
    let text = std::str::from_utf8(&stdout)
        .map_err(|_| bridge_error(ProjectBridgeErrorKind::ProjectFilesystemUnavailable))?;
    let text = text
        .strip_suffix("\r\n")
        .or_else(|| text.strip_suffix('\n'))
        .unwrap_or(text);
    if !safe_text(text, MAX_GIT_OUTPUT_BYTES) || text.nfc().ne(text.chars()) {
        return Err(bridge_error(
            ProjectBridgeErrorKind::ProjectFilesystemUnavailable,
        ));
    }
    Ok(text.to_owned())
}

fn configure_project_git_environment(command: &mut Command) {
    command
        .env_clear()
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "Never")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", null_device())
        .env("GIT_CONFIG_COUNT", "0")
        .env("GIT_ATTR_NOSYSTEM", "1")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_PAGER", "cat")
        .env("LC_ALL", "C")
        .env("LANG", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
}

fn spawn_bounded_git_reader(
    stream: Box<dyn Read + Send>,
) -> thread::JoinHandle<io::Result<Vec<u8>>> {
    thread::spawn(move || read_bounded_git_stream(stream))
}

fn read_bounded_git_stream(mut stream: impl Read) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(MAX_GIT_OUTPUT_BYTES.min(8 * 1_024));
    let mut buffer = [0_u8; 4 * 1_024];
    loop {
        let count = stream.read(&mut buffer)?;
        if count == 0 {
            return Ok(bytes);
        }
        if bytes
            .len()
            .checked_add(count)
            .is_none_or(|total| total > MAX_GIT_OUTPUT_BYTES)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "project Git output limit exceeded",
            ));
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
}

fn join_bounded_git_reader(
    reader: Option<thread::JoinHandle<io::Result<Vec<u8>>>>,
) -> ProjectBridgeResult<Vec<u8>> {
    reader
        .ok_or_else(|| bridge_error(ProjectBridgeErrorKind::ProjectFilesystemUnavailable))?
        .join()
        .map_err(|_| bridge_error(ProjectBridgeErrorKind::ProjectFilesystemUnavailable))?
        .map_err(|_| bridge_error(ProjectBridgeErrorKind::ProjectFilesystemUnavailable))
}

fn wait_for_supervised_child(
    child: &mut SupervisedDuplexChild,
    deadline: Instant,
) -> ProjectBridgeResult<ExitStatus> {
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|_| bridge_error(ProjectBridgeErrorKind::ProjectFilesystemUnavailable))?
        {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            return Err(bridge_error(ProjectBridgeErrorKind::DeadlineExceeded));
        }
        thread::sleep(Duration::from_millis(5));
    }
}

#[cfg(windows)]
fn null_device() -> OsString {
    OsString::from("NUL")
}

#[cfg(not(windows))]
fn null_device() -> OsString {
    OsString::from("/dev/null")
}

fn physical_digest(
    schema: &'static str,
    identity: PhysicalIdentity,
) -> ProjectBridgeResult<ContentDigest> {
    bridge_digest(
        schema,
        &CanonicalValue::Object(vec![
            (
                "device".to_owned(),
                CanonicalValue::String(identity.device.to_string()),
            ),
            (
                "file".to_owned(),
                CanonicalValue::String(identity.file.to_string()),
            ),
        ]),
    )
}

fn paired_physical_digest(
    schema: &'static str,
    first: PhysicalIdentity,
    second: PhysicalIdentity,
) -> ProjectBridgeResult<ContentDigest> {
    bridge_digest(
        schema,
        &CanonicalValue::Object(vec![
            (
                "first_device".to_owned(),
                CanonicalValue::String(first.device.to_string()),
            ),
            (
                "first_file".to_owned(),
                CanonicalValue::String(first.file.to_string()),
            ),
            (
                "second_device".to_owned(),
                CanonicalValue::String(second.device.to_string()),
            ),
            (
                "second_file".to_owned(),
                CanonicalValue::String(second.file.to_string()),
            ),
        ]),
    )
}

fn ref_storage_digest(
    common_directory: PhysicalIdentity,
    reference: &str,
) -> ProjectBridgeResult<ContentDigest> {
    bridge_digest(
        "lattice.project-bridge.primary-ref-identity",
        &CanonicalValue::Object(vec![
            (
                "common_directory_device".to_owned(),
                CanonicalValue::String(common_directory.device.to_string()),
            ),
            (
                "common_directory_file".to_owned(),
                CanonicalValue::String(common_directory.file.to_string()),
            ),
            (
                "symbolic_ref_target".to_owned(),
                CanonicalValue::String(reference.to_owned()),
            ),
        ]),
    )
}

fn zero_digest() -> ProjectBridgeResult<ContentDigest> {
    ContentDigest::from_sha256("0".repeat(64))
        .map_err(|_| bridge_error(ProjectBridgeErrorKind::ProjectFilesystemUnavailable))
}

fn bridge_digest(
    schema: &'static str,
    value: &CanonicalValue,
) -> ProjectBridgeResult<ContentDigest> {
    let domain = HashDomain::new(schema, "1.0")
        .map_err(|_| bridge_error(ProjectBridgeErrorKind::ProjectRegistryRejected))?;
    let digest = canonical_sha256(&domain, value)
        .map_err(|_| bridge_error(ProjectBridgeErrorKind::ProjectRegistryRejected))?;
    ContentDigest::from_sha256(digest.to_hex())
        .map_err(|_| bridge_error(ProjectBridgeErrorKind::ProjectRegistryRejected))
}

fn resolve_registry_authority(
    database: &DeliveryDatabaseBinding,
    password: &str,
    deadline: Instant,
    store_authority: &StoreAuthorityHead,
    project: &CatalogProject,
    observation: &RepositoryObservation,
) -> ProjectBridgeResult<ResolvedProjectAuthority> {
    ensure_before(deadline)?;
    let target = MigrationTarget::new(database.database_name(), database.run_id())
        .map_err(|_| bridge_error(ProjectBridgeErrorKind::ProjectRegistryRejected))?;
    let client = connect_fixed_runtime_client(database, password, deadline)
        .map_err(|_| bridge_error(ProjectBridgeErrorKind::ProjectRegistryUnavailable))?;
    let mut registry = PostgresProjectRegistry::new(client, &target).map_err(map_registry_error)?;
    let loaded = registry.load().map_err(map_registry_error)?;
    let current = loaded.state().project(&project.id);
    let command = if let Some(current) = current {
        validate_current_registry_project(current)?;

        // An identical live observation is already formal Registry truth.
        // Reuse it without minting another Registry revision. The outer
        // resolver performs one fresh Control/Git pass after a currentness
        // collision, pinned to this same selected project ID.
        if current.observation() == observation {
            return Ok(resolved_project_authority(project, current));
        }

        let head = current.authority().head();
        RegistryCommand::observe(
            registry_command_id("observe", &project.id, observation, Some(&head))?,
            project.id.clone(),
            head,
            observation.clone(),
        )
    } else {
        RegistryCommand::register(
            registry_command_id("register", &project.id, observation, None)?,
            project.id.clone(),
            ProjectClass::UserProject,
            observation.clone(),
        )
    };
    let execution = registry
        .execute(command, store_authority.clone())
        .map_err(|error| {
            if registry_error_is_retryable_currentness(error.kind()) {
                bridge_error(ProjectBridgeErrorKind::ProjectRegistryConflict)
            } else {
                map_registry_error(error)
            }
        })?;
    match execution.semantic_receipt().outcome() {
        RegistryCommandOutcome::Applied => {}
        RegistryCommandOutcome::Denied(denial)
            if registry_denial_is_retryable_currentness(&denial) =>
        {
            return Err(bridge_error(
                ProjectBridgeErrorKind::ProjectRegistryConflict,
            ));
        }
        RegistryCommandOutcome::Denied(denial) => return Err(map_registry_denial(&denial)),
        RegistryCommandOutcome::Blocked(_) => {
            return Err(bridge_error(
                ProjectBridgeErrorKind::ProjectIdentityCollision,
            ));
        }
    }
    if !execution.semantic_receipt().drift().is_empty() {
        return Err(bridge_error(ProjectBridgeErrorKind::ProjectIdentityChanged));
    }
    let semantic_authority = execution
        .semantic_receipt()
        .authority()
        .ok_or_else(|| bridge_error(ProjectBridgeErrorKind::ProjectRegistryRejected))?;
    if semantic_authority.lifecycle() != ProjectLifecycle::Active {
        return Err(bridge_error(ProjectBridgeErrorKind::ProjectIdentityChanged));
    }

    // Re-read the complete verified Registry after commit. The returned head
    // is therefore an owner lookup, not a projection of historical receipt
    // bytes retained by the caller.
    let reloaded = registry.load().map_err(map_registry_error)?;
    let current = reloaded
        .state()
        .project(&project.id)
        .ok_or_else(|| bridge_error(ProjectBridgeErrorKind::ProjectRegistryRejected))?;
    validate_current_registry_project(current)?;
    if current.observation() != observation
        || current.authority().head() != semantic_authority.head()
    {
        return Err(bridge_error(
            ProjectBridgeErrorKind::ProjectRegistryConflict,
        ));
    }
    Ok(resolved_project_authority(project, current))
}

fn validate_current_registry_project(
    current: &RegistryProjectProjection,
) -> ProjectBridgeResult<()> {
    if current.project_class() != ProjectClass::UserProject {
        return Err(bridge_error(
            ProjectBridgeErrorKind::ProjectIdentityCollision,
        ));
    }
    if current.authority().lifecycle() != ProjectLifecycle::Active
        || current.pending_observation().is_some()
        || !current.drift().is_empty()
    {
        return Err(bridge_error(ProjectBridgeErrorKind::ProjectInactive));
    }
    Ok(())
}

fn resolved_project_authority(
    project: &CatalogProject,
    current: &RegistryProjectProjection,
) -> ResolvedProjectAuthority {
    ResolvedProjectAuthority {
        display_name: project.name.clone(),
        canonical_path: project.canonical_path.clone(),
        authority: current.authority().clone(),
        current_head: current.authority().head(),
    }
}

const fn registry_error_is_retryable_currentness(kind: PostgresProjectRegistryErrorKind) -> bool {
    matches!(kind, PostgresProjectRegistryErrorKind::CheckpointChanged)
}

const fn registry_denial_is_retryable_currentness(denial: &RegistryDenial) -> bool {
    matches!(
        denial,
        RegistryDenial::StaleHead | RegistryDenial::DuplicateIdentity { .. }
    )
}

fn registry_command_id(
    action: &'static str,
    project_id: &ProjectId,
    observation: &RepositoryObservation,
    expected_head: Option<&ProjectAuthorityHead>,
) -> ProjectBridgeResult<CommandId> {
    let request = CanonicalValue::Object(vec![
        (
            "action".to_owned(),
            CanonicalValue::String(action.to_owned()),
        ),
        (
            "expected_receipt_digest".to_owned(),
            expected_head.map_or(CanonicalValue::Null, |head| {
                CanonicalValue::String(head.receipt_digest().as_str().to_owned())
            }),
        ),
        (
            "observation_digest".to_owned(),
            CanonicalValue::String(observation.digest().as_str().to_owned()),
        ),
        (
            "project_id".to_owned(),
            CanonicalValue::String(project_id.as_str().to_owned()),
        ),
    ]);
    let digest = bridge_digest("lattice.project-bridge.registry-command", &request)?;
    CommandId::new(format!("control-bridge-{action}-{}", digest.as_str()))
        .map_err(|_| bridge_error(ProjectBridgeErrorKind::ProjectRegistryRejected))
}

fn map_registry_denial(denial: &RegistryDenial) -> ProjectBridgeError {
    match denial {
        RegistryDenial::DuplicateIdentity { .. } => {
            bridge_error(ProjectBridgeErrorKind::ProjectIdentityCollision)
        }
        RegistryDenial::UnknownProject | RegistryDenial::StaleHead => {
            bridge_error(ProjectBridgeErrorKind::ProjectRegistryConflict)
        }
        RegistryDenial::LifecycleBlocked { .. } => {
            bridge_error(ProjectBridgeErrorKind::ProjectInactive)
        }
        RegistryDenial::ReconciliationDecisionMismatch { .. }
        | RegistryDenial::PendingObservationMismatch
        | RegistryDenial::RevisionOverflow => {
            bridge_error(ProjectBridgeErrorKind::ProjectRegistryRejected)
        }
    }
}

fn map_registry_error(
    error: lattice_postgres_store::PostgresProjectRegistryError,
) -> ProjectBridgeError {
    match error.kind() {
        PostgresProjectRegistryErrorKind::Unavailable
        | PostgresProjectRegistryErrorKind::TransactionFailed
        | PostgresProjectRegistryErrorKind::SerializationExhausted
        | PostgresProjectRegistryErrorKind::CommitOutcomeUnknown => {
            bridge_error(ProjectBridgeErrorKind::ProjectRegistryUnavailable)
        }
        PostgresProjectRegistryErrorKind::AuthorityMismatch
        | PostgresProjectRegistryErrorKind::CheckpointChanged => {
            bridge_error(ProjectBridgeErrorKind::ProjectRegistryConflict)
        }
        PostgresProjectRegistryErrorKind::AdmissionDenied => {
            bridge_error(ProjectBridgeErrorKind::ProjectInactive)
        }
        _ => bridge_error(ProjectBridgeErrorKind::ProjectRegistryRejected),
    }
}

fn remaining(deadline: Instant) -> ProjectBridgeResult<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| bridge_error(ProjectBridgeErrorKind::DeadlineExceeded))
}

fn ensure_before(deadline: Instant) -> ProjectBridgeResult<()> {
    remaining(deadline).map(|_| ())
}

const fn bridge_error(kind: ProjectBridgeErrorKind) -> ProjectBridgeError {
    ProjectBridgeError { kind }
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicU64, Ordering};

    use serde_json::json;

    use super::*;

    static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn control_origin_is_exact_http_ipv4_loopback_only() {
        assert_eq!(
            parse_control_origin(DEFAULT_CONTROL_ORIGIN),
            Some(ControlOrigin { port: 4317 })
        );
        assert_eq!(
            parse_control_origin("http://127.0.0.1:49123"),
            Some(ControlOrigin { port: 49123 })
        );
        for rejected in [
            "http://localhost:4317",
            "https://127.0.0.1:4317",
            "http://127.0.0.2:4317",
            "http://user@127.0.0.1:4317",
            "http://127.0.0.1:4317/api/state",
            "http://127.0.0.1:0",
            "http://127.0.0.1:04317",
        ] {
            assert_eq!(parse_control_origin(rejected), None, "accepted {rejected}");
        }
    }

    #[test]
    fn selector_excludes_legacy_and_never_guesses_ambiguous_projects() {
        let state = json!({"projects": [
            locator("11111111-1111-1111-1111-111111111111", "Same", true),
            locator("22222222-2222-2222-2222-222222222222", "Same", true),
            locator("33333333-3333-3333-3333-333333333333", "Legacy", false)
        ]});
        let projects = parse_catalog_state(&state).expect("valid state");
        let by_name = ProjectSelector::new(None, Some("Same")).expect("selector");
        assert_eq!(
            select_catalog_project(&projects, &by_name).map_err(ProjectBridgeError::kind),
            Err(ProjectBridgeErrorKind::ProjectAmbiguous)
        );
        let legacy = ProjectSelector::new(Some("33333333-3333-3333-3333-333333333333"), None)
            .expect("selector");
        assert_eq!(
            select_catalog_project(&projects, &legacy).map_err(ProjectBridgeError::kind),
            Err(ProjectBridgeErrorKind::ProjectNotRegistered)
        );
        let no_selector = ProjectSelector::new(None, None).expect("selector");
        assert_eq!(
            select_catalog_project(&projects, &no_selector).map_err(ProjectBridgeError::kind),
            Err(ProjectBridgeErrorKind::ProjectAmbiguous)
        );
    }

    #[test]
    fn selector_free_admission_counts_only_formally_eligible_projects() {
        let eligible_id = "11111111-1111-1111-1111-111111111111";
        let legacy_id = "22222222-2222-2222-2222-222222222222";
        let state = json!({"projects": [
            locator(eligible_id, "Registered", true),
            locator(legacy_id, "Legacy", false)
        ]});
        let projects = parse_catalog_state(&state).expect("valid state");

        let no_selector = ProjectSelector::new(None, None).expect("selector");
        assert_eq!(
            select_catalog_project(&projects, &no_selector)
                .expect("one eligible project")
                .id,
            eligible_id
        );

        let legacy = ProjectSelector::new(Some(legacy_id), None).expect("selector");
        assert_eq!(
            select_catalog_project(&projects, &legacy).map_err(ProjectBridgeError::kind),
            Err(ProjectBridgeErrorKind::ProjectNotRegistered)
        );
    }

    #[test]
    fn exact_id_and_unique_name_select_only_complete_catalog_rows() {
        let state = json!({"projects": [
            locator("11111111-1111-1111-1111-111111111111", "One", true),
            locator("22222222-2222-2222-2222-222222222222", "Two", true)
        ]});
        let projects = parse_catalog_state(&state).expect("valid state");
        let by_id = ProjectSelector::new(Some("22222222-2222-2222-2222-222222222222"), None)
            .expect("selector");
        assert_eq!(
            select_catalog_project(&projects, &by_id)
                .expect("id match")
                .name,
            "Two"
        );
        let by_name = ProjectSelector::new(None, Some("One")).expect("selector");
        assert_eq!(
            select_catalog_project(&projects, &by_name)
                .expect("name match")
                .id
                .as_str(),
            "11111111-1111-1111-1111-111111111111"
        );
    }

    #[test]
    fn catalog_replay_rejects_uniqueness_drift_before_registry_resolution() {
        let first_state = json!({"projects": [
            locator("11111111-1111-1111-1111-111111111111", "Same", true)
        ]});
        let replay_state = json!({"projects": [
            locator("11111111-1111-1111-1111-111111111111", "Same", true),
            locator("22222222-2222-2222-2222-222222222222", "Same", true)
        ]});
        let selector = ProjectSelector::new(None, Some("Same")).expect("selector");
        let first = select_catalog_project(
            &parse_catalog_state(&first_state).expect("first catalog"),
            &selector,
        )
        .expect("initially unique");

        assert_eq!(
            verify_catalog_replay(&first_state, &replay_state, &selector, &first)
                .map_err(ProjectBridgeError::kind),
            Err(ProjectBridgeErrorKind::ProjectIdentityChanged)
        );
        assert_eq!(
            verify_catalog_replay(&first_state, &first_state, &selector, &first)
                .expect("byte-equal replay"),
            first
        );
    }

    #[test]
    fn catalog_replay_ignores_unrelated_control_state_and_project_order() {
        let first_state = json!({
            "codexConnected": false,
            "receiptCount": 1,
            "workItems": [{"id": "first"}],
            "projects": [
                locator("11111111-1111-1111-1111-111111111111", "One", true),
                locator("22222222-2222-2222-2222-222222222222", "Two", true)
            ]
        });
        let replay_state = json!({
            "codexConnected": true,
            "receiptCount": 99,
            "workItems": [{"id": "unrelated"}, {"id": "changed"}],
            "projects": [
                locator("22222222-2222-2222-2222-222222222222", "Two", true),
                locator("11111111-1111-1111-1111-111111111111", "One", true)
            ]
        });
        let selector = ProjectSelector::new(None, Some("One")).expect("selector");
        let first = select_catalog_project(
            &parse_catalog_state(&first_state).expect("first catalog"),
            &selector,
        )
        .expect("unique selected project");

        assert_eq!(
            verify_catalog_replay(&first_state, &replay_state, &selector, &first)
                .expect("unchanged catalog projection"),
            first
        );
    }

    #[test]
    fn catalog_replay_ignores_legacy_only_drift() {
        let eligible_id = "11111111-1111-1111-1111-111111111111";
        let first_state = json!({"projects": [
            locator(eligible_id, "Registered", true),
            locator("22222222-2222-2222-2222-222222222222", "Old legacy", false)
        ]});
        let replay_state = json!({"projects": [
            locator("33333333-3333-3333-3333-333333333333", "New legacy", false),
            locator(eligible_id, "Registered", true)
        ]});
        let selector = ProjectSelector::new(None, None).expect("selector");
        let first = select_catalog_project(
            &parse_catalog_state(&first_state).expect("first catalog"),
            &selector,
        )
        .expect("one eligible project");

        assert_eq!(
            verify_catalog_replay(&first_state, &replay_state, &selector, &first)
                .expect("legacy-only drift cannot alter formal selection"),
            first
        );
    }

    #[test]
    fn registry_currentness_retry_is_once_only_and_cannot_retarget_a_project() {
        assert_eq!(MAX_REGISTRY_CURRENTNESS_RETRIES, 1);
        assert!(registry_error_is_retryable_currentness(
            PostgresProjectRegistryErrorKind::CheckpointChanged
        ));
        assert!(!registry_error_is_retryable_currentness(
            PostgresProjectRegistryErrorKind::AuthorityMismatch
        ));
        assert!(registry_denial_is_retryable_currentness(
            &RegistryDenial::StaleHead
        ));
        assert!(registry_denial_is_retryable_currentness(
            &RegistryDenial::DuplicateIdentity {
                dimension: lattice_project_registry::IdentityDimension::ProjectId,
                existing_project_id: ProjectId::new("11111111-1111-1111-1111-111111111111")
                    .expect("project id"),
            }
        ));
        assert!(!registry_denial_is_retryable_currentness(
            &RegistryDenial::UnknownProject
        ));

        let mut effective = None;
        retain_effective_project_id(&mut effective, "project-a").expect("first selection");
        retain_effective_project_id(&mut effective, "project-a").expect("same-project retry");
        assert_eq!(effective.as_deref(), Some("project-a"));
        assert_eq!(
            retain_effective_project_id(&mut effective, "project-b")
                .map_err(ProjectBridgeError::kind),
            Err(ProjectBridgeErrorKind::ProjectIdentityChanged)
        );
    }

    #[test]
    fn unsupported_control_names_do_not_poison_unrelated_exact_selection_or_get_guessed() {
        let supported_id = "11111111-1111-1111-1111-111111111111";
        let unsupported_id = "legacy-project-id";
        let state = json!({"projects": [
            locator(supported_id, "One", true),
            locator(unsupported_id, &"x".repeat(65), true)
        ]});
        let projects = parse_catalog_state(&state).expect("Control-valid state");

        let by_id = ProjectSelector::new(Some(supported_id), None).expect("selector");
        assert_eq!(
            select_catalog_project(&projects, &by_id)
                .expect("unrelated supported project")
                .id
                .as_str(),
            supported_id
        );
        let unsupported = ProjectSelector::new(Some(unsupported_id), None).expect("selector");
        assert_eq!(
            select_catalog_project(&projects, &unsupported).map_err(ProjectBridgeError::kind),
            Err(ProjectBridgeErrorKind::ProjectNameUnsupported)
        );
        let no_selector = ProjectSelector::new(None, None).expect("selector");
        assert_eq!(
            select_catalog_project(&projects, &no_selector).map_err(ProjectBridgeError::kind),
            Err(ProjectBridgeErrorKind::ProjectAmbiguous)
        );
    }

    #[test]
    fn secret_shaped_control_ids_never_reach_registry_or_poison_unrelated_selection() {
        let supported_id = "11111111-1111-1111-1111-111111111111";
        let unsupported_id = "sk-do-not-use";
        let state = json!({"projects": [
            locator(supported_id, "One", true),
            locator(unsupported_id, "Secret-shaped ID", true)
        ]});
        let projects = parse_catalog_state(&state).expect("Control-valid state");

        assert_eq!(
            ProjectSelector::new(Some(unsupported_id), None).map_err(ProjectBridgeError::kind),
            Err(ProjectBridgeErrorKind::InvalidSelector)
        );
        let by_supported_id = ProjectSelector::new(Some(supported_id), None).expect("selector");
        assert_eq!(
            select_catalog_project(&projects, &by_supported_id)
                .expect("unrelated supported project")
                .id
                .as_str(),
            supported_id
        );
        let by_unsupported_name =
            ProjectSelector::new(None, Some("Secret-shaped ID")).expect("selector");
        assert_eq!(
            select_catalog_project(&projects, &by_unsupported_name)
                .map_err(ProjectBridgeError::kind),
            Err(ProjectBridgeErrorKind::ProjectIdUnsupported)
        );
        let no_selector = ProjectSelector::new(None, None).expect("selector");
        assert_eq!(
            select_catalog_project(&projects, &no_selector).map_err(ProjectBridgeError::kind),
            Err(ProjectBridgeErrorKind::ProjectAmbiguous)
        );

        let only_unsupported = json!({"projects": [
            locator(unsupported_id, "Secret-shaped ID", true)
        ]});
        let projects = parse_catalog_state(&only_unsupported).expect("Control-valid state");
        assert_eq!(
            select_catalog_project(&projects, &no_selector).map_err(ProjectBridgeError::kind),
            Err(ProjectBridgeErrorKind::ProjectIdUnsupported)
        );
    }

    #[test]
    fn noncanonical_legacy_control_id_does_not_poison_supported_exact_selection() {
        let supported_id = "11111111-1111-1111-1111-111111111111";
        let unsupported_id = "LEGACY/42";
        let state = json!({"projects": [
            locator(supported_id, "One", true),
            locator(unsupported_id, "Legacy-shaped ID", true)
        ]});
        let projects = parse_catalog_state(&state).expect("Control-valid bounded legacy ID");

        let by_supported_id = ProjectSelector::new(Some(supported_id), None).expect("selector");
        assert_eq!(
            select_catalog_project(&projects, &by_supported_id)
                .expect("unrelated supported project")
                .id,
            supported_id
        );
        let by_unsupported_name =
            ProjectSelector::new(None, Some("Legacy-shaped ID")).expect("selector");
        assert_eq!(
            select_catalog_project(&projects, &by_unsupported_name)
                .map_err(ProjectBridgeError::kind),
            Err(ProjectBridgeErrorKind::ProjectIdUnsupported)
        );
        let no_selector = ProjectSelector::new(None, None).expect("selector");
        assert_eq!(
            select_catalog_project(&projects, &no_selector).map_err(ProjectBridgeError::kind),
            Err(ProjectBridgeErrorKind::ProjectAmbiguous)
        );
    }

    #[test]
    fn non_nfc_control_name_participates_in_name_ambiguity_without_becoming_task_data() {
        let state = json!({"projects": [
            locator("11111111-1111-1111-1111-111111111111", "Caf\u{e9}", true),
            locator("22222222-2222-2222-2222-222222222222", "Cafe\u{301}", true)
        ]});
        let projects = parse_catalog_state(&state).expect("Control-valid state");
        let selector = ProjectSelector::new(None, Some("Caf\u{e9}")).expect("selector");
        assert_eq!(
            select_catalog_project(&projects, &selector).map_err(ProjectBridgeError::kind),
            Err(ProjectBridgeErrorKind::ProjectAmbiguous)
        );

        let only_unsupported = json!({"projects": [locator(
            "22222222-2222-2222-2222-222222222222",
            "Cafe\u{301}",
            true
        )]});
        let projects = parse_catalog_state(&only_unsupported).expect("Control-valid state");
        assert_eq!(
            select_catalog_project(&projects, &selector).map_err(ProjectBridgeError::kind),
            Err(ProjectBridgeErrorKind::ProjectNameUnsupported)
        );
    }

    #[test]
    fn complete_detail_rejects_failed_or_non_repository_observations() {
        let base = detail("11111111-1111-1111-1111-111111111111", "One");
        assert!(parse_catalog_detail(&base, None).is_ok());
        let mut failed = base.clone();
        failed["last_refresh_failure"] = json!({"code": "FAILED"});
        assert_eq!(
            parse_catalog_detail(&failed, None).map_err(ProjectBridgeError::kind),
            Err(ProjectBridgeErrorKind::ProjectObservationUnavailable)
        );
        let mut detached = base;
        detached["git_observation"]["detached"] = Value::Bool(true);
        assert_eq!(
            parse_catalog_detail(&detached, None).map_err(ProjectBridgeError::kind),
            Err(ProjectBridgeErrorKind::ProjectObservationUnavailable)
        );
    }

    #[test]
    fn http_reader_requires_bounded_content_length_json_from_loopback() {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind loopback");
        let port = listener.local_addr().expect("address").port();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut request = [0_u8; 512];
            let read = stream.read(&mut request).expect("read request");
            let request = std::str::from_utf8(&request[..read]).expect("request text");
            assert!(request.starts_with("GET /api/state HTTP/1.1\r\n"));
            assert!(request.contains(&format!("Host: 127.0.0.1:{port}\r\n")));
            let body = br#"{"projects":[]}"#;
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .expect("headers");
            stream.write_all(body).expect("body");
        });
        let result = control_get_json(
            ControlOrigin { port },
            "/api/state",
            Instant::now() + Duration::from_secs(5),
        )
        .expect("loopback response");
        assert_eq!(result, json!({"projects": []}));
        server.join().expect("server");
    }

    #[test]
    fn physical_and_ref_identities_are_domain_separated_and_stable() {
        let common = PhysicalIdentity {
            device: 7,
            file: 11,
        };
        assert_eq!(
            ref_storage_digest(common, "refs/heads/main").expect("loose"),
            ref_storage_digest(common, "refs/heads/main").expect("packed")
        );
        assert_ne!(
            ref_storage_digest(common, "refs/heads/main").expect("main"),
            ref_storage_digest(common, "refs/heads/other").expect("other")
        );
        assert_ne!(
            physical_digest("lattice.project-bridge.root-identity", common).expect("root"),
            physical_digest("lattice.project-bridge.repository-identity", common)
                .expect("repository")
        );
    }

    #[cfg(windows)]
    #[test]
    fn project_git_safe_directory_normalizes_only_an_exact_wsl_unc_root() {
        assert_eq!(
            git_safe_directory_value(Path::new(
                r"\\?\UNC\wsl.localhost\Ubuntu\home\lattice\repository"
            ))
            .expect("exact Git-compatible WSL UNC root"),
            "//wsl.localhost/Ubuntu/home/lattice/repository"
        );
        assert!(git_safe_directory_value(Path::new("*")).is_err());
        assert!(
            git_safe_directory_value(Path::new(r"\\?\UNC\wsl.localhost\Ubuntu\home\lattice\*"))
                .is_err()
        );
    }

    #[cfg(windows)]
    #[test]
    fn project_git_child_cwd_converts_verbatim_wsl_unc_for_supervisor() {
        let child_path = windows_child_directory_path(Path::new(
            r"\\?\UNC\wsl.localhost\Ubuntu\home\lattice\repository",
        ))
        .expect("supervisor-compatible WSL UNC child cwd");
        assert_eq!(
            child_path,
            PathBuf::from(r"\\wsl.localhost\Ubuntu\home\lattice\repository")
        );
        let mut command = Command::new(test_cmd_executable());
        command.current_dir(&child_path);
        assert_eq!(command.get_current_dir(), Some(child_path.as_path()));
    }

    #[test]
    fn project_git_safe_directory_is_captured_exact_and_precedes_the_subcommand() {
        let root = env::temp_dir().join(format!(
            "lattice-project-safe-directory-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let substituted = root.with_extension("substituted");
        fs::create_dir(&root).expect("captured root");
        fs::create_dir(&substituted).expect("substituted root");
        let mut captured = capture_directory(&root).expect("capture exact root handle");
        let safe_directory =
            git_safe_directory_value(&captured.canonical_path).expect("Git-compatible exact root");
        let arguments = project_git_arguments(&captured, ["rev-parse", "--show-toplevel"])
            .expect("captured safe.directory arguments");
        assert_eq!(arguments[0], OsStr::new("-c"));
        assert_eq!(
            arguments[1],
            OsString::from(format!("safe.directory={safe_directory}"))
        );
        assert_eq!(arguments[2], OsStr::new("rev-parse"));
        assert_eq!(arguments[3], OsStr::new("--show-toplevel"));

        captured.canonical_path = substituted.clone();
        assert_eq!(
            project_git_arguments(&captured, ["status"]).map_err(ProjectBridgeError::kind),
            Err(ProjectBridgeErrorKind::ProjectIdentityChanged)
        );
        drop(captured);
        fs::remove_dir_all(root).expect("remove captured root");
        fs::remove_dir_all(substituted).expect("remove substituted root");
    }

    #[cfg(windows)]
    #[test]
    fn project_git_safe_directory_seals_replay_to_spawn_directory_substitution() {
        let Some(git) = test_git_executable() else {
            panic!("git executable is required for project bridge tests");
        };
        let original = TestRepository::new(&git);
        let substitute = TestRepository::new(&git);
        substitute.git(&["checkout", "-b", "attacker"]);
        let moved = original.root.with_extension("moved-after-replay");
        let captured = capture_directory(&original.root).expect("capture exact repository root");
        let git_identity =
            ManagedFileIdentity::capture(&git, MAX_PROJECT_BRIDGE_GIT_BYTES).expect("Git identity");
        let substitution_succeeded = std::cell::Cell::new(false);
        let output = git_stdout_from_captured_with_hooks(
            &git,
            &git_identity,
            &captured,
            ["symbolic-ref", "--quiet", "HEAD"],
            Instant::now() + Duration::from_secs(5),
            |_| {},
            || {
                if fs::rename(&original.root, &moved).is_ok() {
                    fs::rename(&substitute.root, &original.root)
                        .expect("install substituted repository after replay");
                    substitution_succeeded.set(true);
                }
            },
        )
        .expect("captured repository remains the only child cwd");
        assert!(!substitution_succeeded.get());
        assert_eq!(output, "refs/heads/main");
        drop(captured);
        assert!(!moved.exists());
    }

    #[cfg(unix)]
    #[test]
    fn project_git_safe_directory_rejects_post_replay_substitution_before_child_effect() {
        use std::os::unix::fs::PermissionsExt;

        let root = env::temp_dir().join(format!(
            "lattice-project-safe-directory-race-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let original = root.join("original");
        let moved = root.join("moved");
        let substitute = root.join("substitute");
        let marker = root.join("child-started");
        let executable = root.join("git-probe");
        fs::create_dir_all(&original).expect("original root");
        fs::create_dir(&substitute).expect("substitute root");
        fs::write(
            &executable,
            format!(
                "#!/bin/sh\nprintf child > {:?}\nprintf refs/heads/main\n",
                marker
            ),
        )
        .expect("probe executable");
        let mut permissions = fs::metadata(&executable)
            .expect("probe metadata")
            .permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&executable, permissions).expect("probe permissions");
        let captured = capture_directory(&original).expect("capture exact repository root");
        let executable_identity =
            ManagedFileIdentity::capture(&executable, MAX_PROJECT_BRIDGE_GIT_BYTES)
                .expect("probe identity");
        let failure = git_stdout_from_captured_with_hooks(
            &executable,
            &executable_identity,
            &captured,
            ["status"],
            Instant::now() + Duration::from_secs(5),
            |_| {},
            || {
                fs::rename(&original, &moved).expect("move captured root after replay");
                fs::rename(&substitute, &original).expect("install substitute after replay");
            },
        )
        .expect_err("post-replay substitution must fail before child spawn");
        assert_eq!(
            failure.kind(),
            ProjectBridgeErrorKind::ProjectIdentityChanged
        );
        assert!(!marker.exists(), "substituted child must never start");
        drop(captured);
        fs::rename(&original, &substitute).expect("remove substituted root");
        fs::rename(&moved, &original).expect("restore captured root");
        fs::remove_dir_all(root).expect("remove race fixture");
    }

    #[test]
    fn real_git_observation_survives_loose_to_packed_ref_change() {
        let Some(git) = test_git_executable() else {
            panic!("git executable is required for project bridge tests");
        };
        let fixture = TestRepository::new(&git);
        let first = inspect_repository(
            &fixture.root,
            &git,
            Instant::now() + Duration::from_secs(30),
        )
        .expect("loose observation");
        fixture.git(&["pack-refs", "--all"]);
        let packed = inspect_repository(
            &fixture.root,
            &git,
            Instant::now() + Duration::from_secs(30),
        )
        .expect("packed observation");
        assert_eq!(first, packed);

        fixture.git(&["checkout", "-b", "other"]);
        let other = inspect_repository(
            &fixture.root,
            &git,
            Instant::now() + Duration::from_secs(30),
        )
        .expect("other branch observation");
        assert_eq!(
            first.canonical_root_identity_digest(),
            other.canonical_root_identity_digest()
        );
        assert_eq!(
            first.repository_identity_digest(),
            other.repository_identity_digest()
        );
        assert_eq!(first.file_identity_digest(), other.file_identity_digest());
        assert_ne!(first.primary_branch(), other.primary_branch());
    }

    #[test]
    fn unreadable_or_non_repository_locator_fails_closed() {
        let Some(git) = test_git_executable() else {
            panic!("git executable is required for project bridge tests");
        };
        let missing = env::temp_dir().join(format!(
            "lattice-project-bridge-missing-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        assert_eq!(
            inspect_repository(&missing, &git, Instant::now() + Duration::from_secs(5))
                .map_err(ProjectBridgeError::kind),
            Err(ProjectBridgeErrorKind::ProjectFilesystemUnavailable)
        );
    }

    #[cfg(windows)]
    #[test]
    fn project_git_clears_hostile_trace_environment_before_any_child_effect() {
        let command = test_cmd_executable();
        let identity = ManagedFileIdentity::capture(&command, MAX_PROJECT_BRIDGE_GIT_BYTES)
            .expect("cmd identity");
        let root = env::temp_dir().join(format!(
            "lattice-project-git-env-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).expect("environment fixture");
        let marker = root.join("hostile-trace-effect.txt");
        let probe = root.join("probe.cmd");
        fs::write(
            &probe,
            format!(
                "@echo off\r\nif defined GIT_TRACE (\r\n  echo effect>\"{}\"\r\n) else (\r\n  echo trusted\r\n)\r\n",
                marker.display()
            ),
        )
        .expect("environment probe");
        let output = git_stdout_with_hooks(
            &command,
            &identity,
            &root,
            [
                OsString::from("/d"),
                OsString::from("/c"),
                probe.into_os_string(),
            ],
            Instant::now() + Duration::from_secs(5),
            |process| {
                process.env("GIT_TRACE", &marker);
                process.env("GIT_TRACE2", &marker);
            },
            || {},
            None,
        )
        .expect("closed environment command");
        assert_eq!(output, "trusted");
        assert!(!marker.exists(), "ambient Git trace must have zero effect");
        fs::remove_dir_all(root).expect("remove environment fixture");
    }

    #[cfg(windows)]
    #[test]
    fn project_git_seal_denies_pre_spawn_aba_write_and_delete() {
        let root = env::temp_dir().join(format!(
            "lattice-project-git-aba-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).expect("ABA fixture");
        let executable = root.join("trusted-cmd.exe");
        fs::copy(test_cmd_executable(), &executable).expect("copy trusted executable");
        let identity = ManagedFileIdentity::capture(&executable, MAX_PROJECT_BRIDGE_GIT_BYTES)
            .expect("copied executable identity");
        let marker = root.join("substitution-effect.txt");
        let write_rejected = std::cell::Cell::new(false);
        let delete_rejected = std::cell::Cell::new(false);
        let output = git_stdout_with_hooks(
            &executable,
            &identity,
            &root,
            ["/d", "/c", "echo trusted"],
            Instant::now() + Duration::from_secs(5),
            |_| {},
            || {
                let write = fs::write(&executable, b"malicious replacement");
                write_rejected.set(write.is_err());
                if write.is_ok() {
                    fs::write(&marker, b"effect").expect("record unsafe substitution");
                }
                delete_rejected.set(fs::remove_file(&executable).is_err());
            },
            None,
        )
        .expect("sealed executable command");
        assert_eq!(output, "trusted");
        assert!(write_rejected.get(), "held handle must deny ABA write");
        assert!(delete_rejected.get(), "held handle must deny ABA delete");
        assert!(
            !marker.exists(),
            "substituted executable effect must be absent"
        );
        fs::remove_dir_all(root).expect("remove ABA fixture");
    }

    #[cfg(windows)]
    #[test]
    fn project_git_timeout_reaps_descendant_and_joins_bounded_pipes() {
        let command = test_cmd_executable();
        let identity = ManagedFileIdentity::capture(&command, MAX_PROJECT_BRIDGE_GIT_BYTES)
            .expect("cmd identity");
        let root = env::temp_dir().join(format!(
            "lattice-project-git-timeout-{}-{}",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).expect("timeout fixture");
        let marker = root.join("late-descendant-effect.txt");
        let ping = PathBuf::from(r"C:\Windows\System32\PING.EXE");
        let descendant = root.join("descendant.cmd");
        let launcher = root.join("launcher.cmd");
        fs::write(
            &descendant,
            format!(
                "@echo off\r\n\"{}\" -n 3 127.0.0.1 >nul\r\necho late>\"{}\"\r\n",
                ping.display(),
                marker.display()
            ),
        )
        .expect("descendant script");
        fs::write(
            &launcher,
            format!(
                "@echo off\r\nstart \"\" /b \"{}\" /d /c \"{}\"\r\n\"{}\" -n 30 127.0.0.1 >nul\r\n",
                command.display(),
                descendant.display(),
                ping.display()
            ),
        )
        .expect("launcher script");
        let failure = git_stdout_with_hooks(
            &command,
            &identity,
            &root,
            [
                OsString::from("/d"),
                OsString::from("/c"),
                launcher.into_os_string(),
            ],
            Instant::now() + Duration::from_millis(250),
            |_| {},
            || {},
            None,
        )
        .expect_err("hanging root and descendant must time out");
        assert_eq!(failure.kind(), ProjectBridgeErrorKind::DeadlineExceeded);
        thread::sleep(Duration::from_secs(3));
        assert!(!marker.exists(), "owned Job descendant must be reaped");
        fs::remove_dir_all(root).expect("remove timeout fixture");
    }

    #[test]
    fn project_git_process_contract_is_sealed_closed_and_supervised() {
        let source = include_str!("project_bridge.rs");
        let transport = source
            .split("fn git_stdout_with_hooks")
            .nth(1)
            .expect("project Git transport")
            .split("#[cfg(windows)]\nfn null_device")
            .next()
            .expect("transport body");
        let seal = transport.find(".seal()").expect("Git executable seal");
        let spawn = transport
            .find("SupervisedDuplexChild::spawn_with_stderr")
            .expect("Job-contained spawn");
        let cleanup = transport
            .find("terminate_and_reap")
            .expect("subtree-zero cleanup");
        assert!(seal < spawn && spawn < cleanup);
        assert!(transport.contains("env_clear()"));
        assert!(transport.contains("spawn_bounded_git_reader(stdout)"));
        assert!(transport.contains("spawn_bounded_git_reader(stderr)"));
        assert!(!transport.contains("std::process::Child"));
    }

    fn locator(id: &str, name: &str, eligible: bool) -> Value {
        if eligible {
            json!({
                "id": id,
                "name": name,
                "canonical_path": test_absolute_path(),
                "schema_version": CATALOG_SCHEMA,
                "record_kind": CATALOG_RECORD_KIND,
                "registry_authority": CATALOG_AUTHORITY,
                "registry_project_id": null,
                "control_project_id": id
            })
        } else {
            json!({
                "id": id,
                "name": name,
                "canonical_path": null,
                "schema_version": null,
                "record_kind": "LEGACY_CONTROL_PROJECT",
                "registry_authority": CATALOG_AUTHORITY,
                "registry_project_id": null,
                "control_project_id": id
            })
        }
    }

    fn detail(id: &str, name: &str) -> Value {
        let path = test_absolute_path();
        json!({
            "id": id,
            "name": name,
            "root_path": path,
            "canonical_path": path,
            "repo_root": path,
            "created_at": "2026-08-26T00:00:00.000Z",
            "updated_at": "2026-08-26T00:00:00.000Z",
            "registered_at": "2026-08-26T00:00:00.000Z",
            "refreshed_at": "2026-08-26T00:00:00.000Z",
            "schema_version": CATALOG_SCHEMA,
            "record_kind": CATALOG_RECORD_KIND,
            "registry_authority": CATALOG_AUTHORITY,
            "registry_project_id": null,
            "control_project_id": id,
            "last_refresh_failure": null,
            "git_observation": {
                "status": "complete",
                "is_repository": true,
                "branch": "main",
                "detached": false,
                "failures": []
            },
            "rule_index": {"status": "complete", "failures": []}
        })
    }

    #[cfg(windows)]
    fn test_absolute_path() -> &'static str {
        r"C:\fixture"
    }

    #[cfg(not(windows))]
    fn test_absolute_path() -> &'static str {
        "/fixture"
    }

    fn test_git_executable() -> Option<PathBuf> {
        if let Some(path) = env::var_os(GIT_EXECUTABLE_ENV).map(PathBuf::from)
            && path.is_absolute()
            && path.is_file()
        {
            return fs::canonicalize(path).ok();
        }
        #[cfg(windows)]
        let output = Command::new("where.exe").arg("git.exe").output().ok()?;
        #[cfg(not(windows))]
        let output = Command::new("sh")
            .args(["-c", "command -v git"])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let first = std::str::from_utf8(&output.stdout).ok()?.lines().next()?;
        fs::canonicalize(first.trim()).ok()
    }

    #[cfg(windows)]
    fn test_cmd_executable() -> PathBuf {
        let output = Command::new("where.exe")
            .arg("cmd.exe")
            .output()
            .expect("locate cmd.exe");
        assert!(output.status.success());
        let path = std::str::from_utf8(&output.stdout)
            .expect("where output")
            .lines()
            .find(|line| !line.trim().is_empty())
            .map(str::trim)
            .expect("cmd path");
        fs::canonicalize(path).expect("canonical cmd path")
    }

    struct TestRepository {
        root: PathBuf,
        git: PathBuf,
    }

    impl TestRepository {
        fn new(git: &Path) -> Self {
            let root = env::temp_dir().join(format!(
                "lattice-project-bridge-{}-{}",
                std::process::id(),
                TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&root).expect("create fixture");
            let fixture = Self {
                root,
                git: git.to_path_buf(),
            };
            fixture.git(&["init", "-b", "main", "."]);
            fixture.git(&["config", "user.email", "acceptance@example.invalid"]);
            fixture.git(&["config", "user.name", "LATTICE Acceptance"]);
            fs::write(fixture.root.join("seed.txt"), b"seed\n").expect("seed");
            fixture.git(&["add", "seed.txt"]);
            fixture.git(&["commit", "-m", "seed"]);
            fixture
        }

        fn git(&self, arguments: &[&str]) {
            let output = Command::new(&self.git)
                .current_dir(&self.root)
                .args(arguments)
                .env("GIT_TERMINAL_PROMPT", "0")
                .output()
                .expect("run git");
            assert!(output.status.success(), "git command failed");
        }
    }

    impl Drop for TestRepository {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}
