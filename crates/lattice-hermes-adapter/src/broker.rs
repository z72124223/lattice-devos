//! Closed command surface for the in-sandbox Codex relay.

use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

#[cfg(windows)]
use std::ffi::OsString;
#[cfg(windows)]
use std::fs::OpenOptions;
#[cfg(windows)]
use std::io::{BufRead, BufReader, Write};
#[cfg(windows)]
use std::process::{Child, ChildStdin, Command, Stdio};
#[cfg(windows)]
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(windows)]
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
#[cfg(windows)]
use std::sync::{Arc, Mutex};
#[cfg(windows)]
use std::thread::{self, JoinHandle};
#[cfg(windows)]
use std::time::{Duration, Instant};
#[cfg(test)]
use std::time::{SystemTime, UNIX_EPOCH};

use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use lattice_contracts::ContentDigest;

#[cfg(windows)]
use crate::codex_proxy::{
    ProductionCodexProxyControl, ProductionCodexProxyDuplex, ProductionCodexProxyProvider,
};
use crate::{HermesAdapterError, HermesAdapterErrorKind, HermesAdapterResult};

const CODEX_VERSION: &str = "codex-cli 0.146.0";
const CODEX_LAUNCHER_SHA256: &str =
    "bc343ba420dc2e2e9f59e6fc5e5bf0aae1cd8c771fc319665241fc9c0271fddb";
const CODEX_SANDBOX_SETUP_SHA256: &str =
    "c12d225b34e7f82cdab6bbc714797abed661f40e158104694953889750121cef";
const CODEX_COMMAND_RUNNER_SHA256: &str =
    "0102fa1820ecd03bb03a991fd2303a1a484118f7da8a71864f88ec94bca61d6d";
const CODEX_PACKAGE_MANIFEST_SHA256: &str =
    "aaa0646d6b615da94187b51efd50c69621a00867761161ae55cc16cfd545bec7";
const CODEX_HOME_OWNERSHIP_MARKER_NAME: &str = ".lattice-codex-home-v1";
const CODEX_HOME_OWNERSHIP_MARKER_BYTES: &[u8] = b"lattice.codex-home.v1\n";
const MAX_CODEX_AUTH_BYTES: u64 = 4 * 1024 * 1024;
const CODEX_CONFIG_LOCK: &str = r#"version = 1
codex_version = "0.146.0"

[config]
model = "gpt-5.3-codex-spark"
model_provider = "openai"
approval_policy = "never"
include_permissions_instructions = false
include_apps_instructions = false
include_collaboration_mode_instructions = false
include_environment_context = false
project_doc_max_bytes = 32768
project_doc_fallback_filenames = []
background_terminal_max_timeout = 300000
hide_agent_reasoning = false
model_reasoning_effort = "low"
web_search = "disabled"

[config.shell_environment_policy]

[config.mcp_servers]

[config.model_providers]

[config.profiles]

[config.history]
persistence = "none"

[config.orchestrator.skills]
enabled = false

[config.orchestrator.mcp]
enabled = false

[config.tools.experimental_request_user_input]
enabled = false

[config.tools.update_plan]
enabled = false

[config.agents]
enabled = false
max_depth = 1
interrupt_message = true

[config.memories]
disable_on_external_context = false
generate_memories = false
use_memories = false
dedicated_tools = false
max_raw_memories_for_consolidation = 256
max_unused_days = 30
max_rollout_age_days = 10
max_rollouts_per_startup = 2
min_rollout_idle_hours = 6
min_rate_limit_remaining_percent = 25

[config.skills]
include_instructions = false

[config.plugins]

[config.marketplaces]

[config.features]
code_mode = false
code_mode_host = false
non_prefixed_mcp_tool_names = false
token_budget = false
rollout_budget = false
current_time_reminder = false
network_proxy = false
apply_patch_freeform = false
apply_patch_streaming_events = false
apps = false
apps_mcp_path_override = false
artifact = false
auth_elicitation = false
browser_use = false
browser_use_external = false
browser_use_full_cdp_access = false
chronicle = false
code_mode_buffered_exec = false
code_mode_only = false
codex_git_commit = false
collaboration_modes = true
computer_use = false
concurrent_reasoning_summaries = false
default_mode_request_user_input = false
deferred_executor = false
deferred_tool_world_state = false
elevated_windows_sandbox = false
enable_fanout = false
enable_mcp_apps = false
enable_request_compression = true
exec_permission_approvals = false
executor_capability_discovery = false
experimental_windows_sandbox = false
external_agent_memory_import = false
external_migration = false
fast_mode = true
goals = false
guardian_approval = true
guardianv2 = false
hooks = false
image_detail_original = false
image_generation = false
in_app_browser = false
in_app_updates = false
item_ids = true
js_repl = false
js_repl_tools_only = false
local_thread_store_compression = false
mcp_2026_07_28 = false
memories = false
mentions_v2 = true
multi_agent = false
multi_agent_mode = false
personality = true
plugin_hooks = false
plugin_sharing = false
plugins = false
prevent_idle_sleep = false
realtime_conversation = false
remote_compaction_v2 = true
remote_control = false
remote_models = false
remote_plugin = false
request_permissions_tool = false
request_rule = false
resize_all_images = true
respect_system_proxy = false
responses_websockets = false
responses_websockets_v2 = false
runtime_metrics = false
search_tool = false
secret_auth_storage = true
shell_snapshot = false
shell_tool = false
shell_zsh_fork = false
skill_env_var_dependency_prompt = false
skill_mcp_dependency_install = false
skill_search = false
sqlite = true
standalone_web_search = false
steer = true
terminal_resize_reflow = true
terminal_visualization_instructions = false
tool_call_mcp_elicitation = false
tool_search = false
tool_search_always_defer_mcp_tools = true
tool_suggest = false
tui_app_server = true
unavailable_dummy_tools = false
undo = false
unified_exec = false
unified_exec_zsh_fork = false
use_agent_identity = false
use_legacy_landlock = false
use_linux_sandbox_bwrap = false
web_search_cached = false
web_search_request = false
workspace_dependencies = false
workspace_owner_usage_nudge = false

[config.features.multi_agent_v2]
enabled = false
max_concurrent_threads_per_session = 4
min_wait_timeout_ms = 10000
max_wait_timeout_ms = 3600000
default_wait_timeout_ms = 30000
root_agent_usage_hint_text = "disabled"
subagent_usage_hint_text = "disabled"
tool_namespace = "collaboration"
hide_spawn_agent_metadata = true
expose_spawn_agent_model_overrides = true
wait_agent_enabled = true
non_code_mode_only = true

[config.apps._default]
enabled = false
"#;
const CODEX_CONFIG_LOCK_RESOLUTION_OVERRIDE: &str =
    "debug.config_lockfile.save_fields_resolved_from_model_catalog=false";
const MAX_CODEX_LAUNCHER_BYTES: u64 = 512 * 1024 * 1024;
const MAX_CODEX_RESOURCE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_CODEX_MANIFEST_BYTES: u64 = 64 * 1024;

/// The only two invocations accepted by the pinned in-sandbox Codex proxy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodexProxyInvocation {
    /// Exact official bundle identity probe.
    Version,
    /// One stdio app-server relay.
    AppServer,
}

impl CodexProxyInvocation {
    /// Parses a complete proxy argv. No flags, aliases, or subcommands are
    /// caller-selectable beyond these exact single-token forms.
    ///
    /// # Errors
    ///
    /// Rejects every command shape other than `--version` and `app-server`.
    pub fn parse<I, S>(arguments: I) -> HermesAdapterResult<Self>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let arguments = arguments.into_iter().collect::<Vec<_>>();
        match arguments.as_slice() {
            [argument] if argument.as_ref() == "--version" => Ok(Self::Version),
            [command, listen, endpoint, strict]
                if command.as_ref() == "app-server"
                    && listen.as_ref() == "--listen"
                    && endpoint.as_ref() == "stdio://"
                    && strict.as_ref() == "--strict-config" =>
            {
                Ok(Self::AppServer)
            }
            _ => Err(HermesAdapterError::new(
                HermesAdapterErrorKind::Configuration,
                "HERMES_CODEX_PROXY_INVOCATION_REJECTED",
            )),
        }
    }
}

/// Exact official Codex identity and fail-closed configuration lock.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CodexBrokerPolicy {
    _private: (),
}

#[allow(clippy::unused_self)]
impl CodexBrokerPolicy {
    pub(crate) const fn official() -> Self {
        Self { _private: () }
    }

    pub(crate) const fn codex_version(self) -> &'static str {
        CODEX_VERSION
    }

    pub(crate) const fn launcher_sha256(self) -> &'static str {
        CODEX_LAUNCHER_SHA256
    }

    pub(crate) const fn sandbox_setup_sha256(self) -> &'static str {
        CODEX_SANDBOX_SETUP_SHA256
    }

    pub(crate) const fn command_runner_sha256(self) -> &'static str {
        CODEX_COMMAND_RUNNER_SHA256
    }

    pub(crate) const fn package_manifest_sha256(self) -> &'static str {
        CODEX_PACKAGE_MANIFEST_SHA256
    }

    /// Exact 0.146.0 strict config admitted by the generated schema and
    /// no-tools preflight.
    pub(crate) const fn config_lock_toml(self) -> &'static str {
        CODEX_CONFIG_LOCK
    }

    /// Fixed post-scrub environment required to disable exec-server selection
    /// and app-server remote control. Normal inherited variables are not part
    /// of this map.
    pub(crate) fn required_child_environment(self) -> BTreeMap<&'static str, &'static str> {
        BTreeMap::from([
            ("CODEX_EXEC_SERVER_URL", "none"),
            ("CODEX_INTERNAL_APP_SERVER_REMOTE_CONTROL_DISABLED", "1"),
        ])
    }

    /// Accepts only a positively observed literal empty model-visible tool
    /// collection. A no-marker response is not evidence of tool absence.
    pub(crate) fn verify_model_visible_tools<I, S>(self, tools: I) -> HermesAdapterResult<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut observed = tools.into_iter();
        if observed.next().is_some() {
            return Err(fatal("HERMES_CODEX_TOOLSET_NOT_EMPTY"));
        }
        Ok(())
    }
}

/// Private result of verifying all four files in the official 0.146.0 bundle.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ReviewedCodexBundle {
    launcher: PathBuf,
    launcher_sha256: String,
    package_manifest_sha256: String,
}

#[allow(clippy::unused_self)]
impl ReviewedCodexBundle {
    pub(crate) const fn version(&self) -> &'static str {
        CODEX_VERSION
    }

    pub(crate) fn launcher_sha256(&self) -> &str {
        &self.launcher_sha256
    }

    pub(crate) fn package_manifest_sha256(&self) -> &str {
        &self.package_manifest_sha256
    }

    pub(crate) fn launcher(&self) -> &Path {
        &self.launcher
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CodexPackageManifest {
    #[serde(rename = "layoutVersion")]
    layout_version: u32,
    version: String,
    target: String,
    variant: String,
    entrypoint: String,
    #[serde(rename = "resourcesDir")]
    resources_dir: String,
    #[serde(rename = "pathDir")]
    path_dir: String,
}

/// Verifies the canonical launcher plus both sandbox resources and the package
/// manifest. A single-file hash is intentionally insufficient.
pub(crate) fn verify_official_codex_bundle(
    launcher: &Path,
) -> HermesAdapterResult<ReviewedCodexBundle> {
    let rejected = || {
        HermesAdapterError::new(
            HermesAdapterErrorKind::Identity,
            "HERMES_CODEX_BUNDLE_IDENTITY_REJECTED",
        )
    };
    if !launcher.is_absolute()
        || launcher.file_name().and_then(|name| name.to_str()) != Some("codex.exe")
    {
        return Err(rejected());
    }
    let bin_root = launcher.parent().ok_or_else(rejected)?;
    if bin_root.file_name().and_then(|name| name.to_str()) != Some("bin") {
        return Err(rejected());
    }
    let bundle_root = bin_root.parent().ok_or_else(rejected)?;
    if bundle_root.file_name().and_then(|name| name.to_str()) != Some("x86_64-pc-windows-msvc") {
        return Err(rejected());
    }
    let canonical_launcher = fs::canonicalize(launcher).map_err(|_| rejected())?;
    let canonical_bundle = fs::canonicalize(bundle_root).map_err(|_| rejected())?;
    if canonical_launcher.parent().and_then(Path::parent) != Some(canonical_bundle.as_path()) {
        return Err(rejected());
    }
    let sandbox_setup = canonical_bundle
        .join("codex-resources")
        .join("codex-windows-sandbox-setup.exe");
    let command_runner = canonical_bundle
        .join("codex-resources")
        .join("codex-command-runner.exe");
    let package_manifest = canonical_bundle.join("codex-package.json");
    for path in [
        canonical_launcher.as_path(),
        sandbox_setup.as_path(),
        command_runner.as_path(),
        package_manifest.as_path(),
    ] {
        reject_reparse_to_boundary(path, &canonical_bundle)?;
    }
    let policy = CodexBrokerPolicy::official();
    let launcher_sha256 = bounded_file_sha256(&canonical_launcher, MAX_CODEX_LAUNCHER_BYTES)?;
    if launcher_sha256 != policy.launcher_sha256()
        || bounded_file_sha256(&sandbox_setup, MAX_CODEX_RESOURCE_BYTES)?
            != policy.sandbox_setup_sha256()
        || bounded_file_sha256(&command_runner, MAX_CODEX_RESOURCE_BYTES)?
            != policy.command_runner_sha256()
    {
        return Err(rejected());
    }
    let manifest_bytes = bounded_file_bytes(&package_manifest, MAX_CODEX_MANIFEST_BYTES)?;
    let package_manifest_sha256 = sha256_bytes(&manifest_bytes);
    if package_manifest_sha256 != policy.package_manifest_sha256() {
        return Err(rejected());
    }
    let manifest: CodexPackageManifest =
        serde_json::from_slice(&manifest_bytes).map_err(|_| rejected())?;
    if manifest.layout_version != 1
        || manifest.version != "0.146.0"
        || manifest.target != "x86_64-pc-windows-msvc"
        || manifest.variant != "codex"
        || manifest.entrypoint != "bin/codex.exe"
        || manifest.resources_dir != "codex-resources"
        || manifest.path_dir != "codex-path"
    {
        return Err(rejected());
    }
    Ok(ReviewedCodexBundle {
        launcher: canonical_launcher,
        launcher_sha256,
        package_manifest_sha256,
    })
}

fn bounded_file_sha256(path: &Path, limit: u64) -> HermesAdapterResult<String> {
    let bytes = bounded_file_bytes(path, limit)?;
    Ok(sha256_bytes(&bytes))
}

fn bounded_file_bytes(path: &Path, limit: u64) -> HermesAdapterResult<Vec<u8>> {
    let rejected = || {
        HermesAdapterError::new(
            HermesAdapterErrorKind::Identity,
            "HERMES_CODEX_BUNDLE_IDENTITY_REJECTED",
        )
    };
    let metadata = fs::symlink_metadata(path).map_err(|_| rejected())?;
    if !metadata.file_type().is_file() || metadata.len() == 0 || metadata.len() > limit {
        return Err(rejected());
    }
    let capacity = usize::try_from(metadata.len()).map_err(|_| rejected())?;
    let mut bytes = Vec::with_capacity(capacity);
    File::open(path)
        .map_err(|_| rejected())?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| rejected())?;
    if bytes.len() != capacity {
        return Err(rejected());
    }
    Ok(bytes)
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

fn reject_reparse_to_boundary(path: &Path, boundary: &Path) -> HermesAdapterResult<()> {
    let rejected = || {
        HermesAdapterError::new(
            HermesAdapterErrorKind::Identity,
            "HERMES_CODEX_BUNDLE_IDENTITY_REJECTED",
        )
    };
    let mut current = path;
    loop {
        let metadata = fs::symlink_metadata(current).map_err(|_| rejected())?;
        if metadata_is_reparse(&metadata) {
            return Err(rejected());
        }
        if current == boundary {
            return Ok(());
        }
        current = current.parent().ok_or_else(rejected)?;
    }
}

#[cfg(windows)]
fn metadata_is_reparse(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn metadata_is_reparse(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

#[cfg(windows)]
const BROKER_HELPER_MAGIC: &[u8] = b"LATTICE_CODEX_BROKER_CANDIDATE_V1\n";
#[cfg(windows)]
const BROKER_HELPER_SCHEMA: &str = "lattice.hermes.codex-broker-helper.v1";
#[cfg(windows)]
const BROKER_CANDIDATE_SCHEMA: &str = "lattice.hermes.codex-broker-candidate.v1";
#[cfg(windows)]
const MAX_BROKER_WIRE_BYTES: usize = 64 * 1024;
const MAX_CODEX_FRAME_BYTES: usize = 1024 * 1024;
#[cfg(windows)]
const MAX_CODEX_STDERR_BYTES: u64 = 8 * 1024 * 1024;
#[cfg(windows)]
const MAX_CODEX_PROXY_STDERR_BYTES: u64 = 64 * 1024;

/// Private host-side configuration for the one-shot official Codex broker.
/// No constructor is exported outside this crate, so an arbitrary helper
/// digest cannot mint a broker receipt.
#[cfg(windows)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexReflectionBrokerConfig {
    broker_helper: PathBuf,
    broker_helper_sha256: String,
    codex_home: PathBuf,
    isolation_root: PathBuf,
    launcher: PathBuf,
    model: String,
    product_root: PathBuf,
}

#[cfg(windows)]
impl CodexReflectionBrokerConfig {
    #[allow(clippy::too_many_arguments)]
    /// Creates a deployment-owned broker configuration.
    ///
    /// The helper digest is a deployment trust input. The resulting receipt
    /// binds it, allowing the private Live gate to compare it with the frozen
    /// runtime manifest rather than trusting the caller at use time.
    ///
    /// # Errors
    ///
    /// Rejects non-absolute paths, an invalid digest, or model drift.
    pub fn new(
        broker_helper: PathBuf,
        broker_helper_sha256: impl Into<String>,
        launcher: PathBuf,
        codex_home: PathBuf,
        isolation_root: PathBuf,
        product_root: PathBuf,
        model: impl Into<String>,
    ) -> HermesAdapterResult<Self> {
        let broker_helper_sha256 = broker_helper_sha256.into();
        let model = model.into();
        if !broker_helper.is_absolute()
            || !launcher.is_absolute()
            || !codex_home.is_absolute()
            || !isolation_root.is_absolute()
            || !product_root.is_absolute()
            || !is_lowercase_sha256(&broker_helper_sha256)
            || model != "gpt-5.3-codex-spark"
        {
            return Err(configuration("HERMES_CODEX_BROKER_CONFIG_REJECTED"));
        }
        Ok(Self {
            broker_helper,
            broker_helper_sha256,
            codex_home,
            isolation_root,
            launcher,
            model,
            product_root,
        })
    }

    /// Seals the exact official bundle, helper, isolated home, config lock,
    /// and scrubbed child environment without starting Codex or a model turn.
    ///
    /// The returned receipt is only a configuration/identity prerequisite.
    /// The production provider revalidates the complete binding immediately
    /// before it opens the one permitted app-server relay.
    ///
    /// # Errors
    ///
    /// Fails closed on deadline, identity, path, home, config, or environment
    /// drift. No child process is started by this method.
    pub fn run_zero_model_preflight(
        &self,
        deadline: Instant,
    ) -> HermesAdapterResult<CodexBrokerPreflightReceipt> {
        if deadline <= Instant::now() {
            return Err(timeout("HERMES_CODEX_BROKER_DEADLINE_EXCEEDED"));
        }
        let policy = CodexBrokerPolicy::official();
        policy.verify_model_visible_tools(std::iter::empty::<&str>())?;
        let reviewed = verify_official_codex_bundle(&self.launcher)?;
        if reviewed.version() != policy.codex_version() {
            return Err(identity("HERMES_CODEX_BUNDLE_IDENTITY_REJECTED"));
        }
        let helper = fs::canonicalize(&self.broker_helper)
            .map_err(|_| identity("HERMES_CODEX_BROKER_HELPER_IDENTITY_REJECTED"))?;
        reject_reparse_to_boundary(&helper, &helper)?;
        let helper_sha256 = bounded_file_sha256(&helper, MAX_CODEX_LAUNCHER_BYTES)?;
        if helper_sha256 != self.broker_helper_sha256 {
            return Err(identity("HERMES_CODEX_BROKER_HELPER_IDENTITY_REJECTED"));
        }
        let codex_home = fs::canonicalize(&self.codex_home)
            .map_err(|_| configuration("HERMES_CODEX_HOME_REJECTED"))?;
        validate_broker_codex_home(&codex_home, &self.product_root)?;
        let (isolation_root, _) =
            crate::validate_isolation_boundary(&self.isolation_root, &self.product_root)?;
        fs::create_dir(&isolation_root)
            .map_err(|_| spawn("HERMES_CODEX_BROKER_ROOT_CREATE_FAILED"))?;
        create_owned_directory(&isolation_root, "empty-work")?;
        create_owned_directory(&isolation_root, "temp")?;
        let config_lock = isolation_root.join("codex-reflection.lock.toml");
        write_new_file(&config_lock, policy.config_lock_toml().as_bytes())?;
        if deadline <= Instant::now()
            || bounded_file_sha256(&helper, MAX_CODEX_LAUNCHER_BYTES)? != helper_sha256
        {
            return Err(identity("HERMES_CODEX_BROKER_HELPER_IDENTITY_REJECTED"));
        }
        let verified = VerifiedCodexProxyConfig::from_config(self.clone())?;
        let receipt_digest = verified.preflight_receipt_digest(&reviewed)?;
        Ok(CodexBrokerPreflightReceipt {
            child_environment_sha256: verified.child_environment_sha256,
            config_lock_sha256: verified.config_lock_sha256,
            helper_sha256: verified.helper_sha256,
            launcher_sha256: reviewed.launcher_sha256().to_owned(),
            receipt_digest,
            #[cfg(test)]
            test_only: false,
        })
    }

    pub(crate) fn into_production_proxy_provider_from_preflight(
        self,
        receipt: &CodexBrokerPreflightReceipt,
        expected_model: &str,
    ) -> HermesAdapterResult<Box<dyn ProductionCodexProxyProvider>> {
        let binding_rejected = || {
            HermesAdapterError::new(
                HermesAdapterErrorKind::CrossBinding,
                "HERMES_CODEX_PROXY_FACTORY_BINDING_REJECTED",
            )
        };
        if expected_model != self.model || expected_model != "gpt-5.3-codex-spark" {
            return Err(binding_rejected());
        }
        receipt.validate_for_containment()?;
        if receipt.helper_sha256 != self.broker_helper_sha256
            || receipt.launcher_sha256 != CODEX_LAUNCHER_SHA256
            || receipt.config_lock_sha256 != sha256_bytes(CODEX_CONFIG_LOCK.as_bytes())
        {
            return Err(binding_rejected());
        }
        let verified = VerifiedCodexProxyConfig::from_config(self)?;
        if verified.helper_sha256 != receipt.helper_sha256
            || verified.config_lock_sha256 != receipt.config_lock_sha256
            || verified.child_environment_sha256 != receipt.child_environment_sha256
        {
            return Err(binding_rejected());
        }
        #[cfg(test)]
        if !receipt.test_only {
            let reviewed = verified.reverify_open()?;
            if verified.preflight_receipt_digest(&reviewed)? != receipt.receipt_digest {
                return Err(binding_rejected());
            }
        }
        #[cfg(not(test))]
        {
            let reviewed = verified.reverify_open()?;
            if verified.preflight_receipt_digest(&reviewed)? != receipt.receipt_digest {
                return Err(binding_rejected());
            }
        }
        Ok(Box::new(OfficialCodexProxyProvider {
            verified,
            control: Arc::new(OwnedCodexProxyControl::new(MAX_CODEX_PROXY_STDERR_BYTES)),
        }))
    }
}

#[cfg(windows)]
#[derive(Clone, Debug, Eq, PartialEq)]
struct VerifiedCodexProxyConfig {
    broker_helper: PathBuf,
    child_environment_sha256: String,
    codex_home: PathBuf,
    config_lock: PathBuf,
    config_lock_sha256: String,
    cwd: PathBuf,
    helper_sha256: String,
    isolation_root: PathBuf,
    launcher: PathBuf,
    model: String,
    product_root: PathBuf,
    temp: PathBuf,
}

#[cfg(windows)]
impl VerifiedCodexProxyConfig {
    fn from_config(config: CodexReflectionBrokerConfig) -> HermesAdapterResult<Self> {
        let identity_rejected = || {
            HermesAdapterError::new(
                HermesAdapterErrorKind::Identity,
                "HERMES_CODEX_PROXY_CONFIG_IDENTITY_REJECTED",
            )
        };
        let broker_helper =
            fs::canonicalize(&config.broker_helper).map_err(|_| identity_rejected())?;
        let launcher = fs::canonicalize(&config.launcher).map_err(|_| identity_rejected())?;
        let codex_home = fs::canonicalize(&config.codex_home).map_err(|_| identity_rejected())?;
        let isolation_root =
            fs::canonicalize(&config.isolation_root).map_err(|_| identity_rejected())?;
        let product_root =
            fs::canonicalize(&config.product_root).map_err(|_| identity_rejected())?;
        let cwd =
            fs::canonicalize(isolation_root.join("empty-work")).map_err(|_| identity_rejected())?;
        let temp =
            fs::canonicalize(isolation_root.join("temp")).map_err(|_| identity_rejected())?;
        let config_lock = fs::canonicalize(isolation_root.join("codex-reflection.lock.toml"))
            .map_err(|_| identity_rejected())?;
        for path in [
            broker_helper.as_path(),
            launcher.as_path(),
            codex_home.as_path(),
            isolation_root.as_path(),
            product_root.as_path(),
            cwd.as_path(),
            temp.as_path(),
            config_lock.as_path(),
        ] {
            crate::reject_link_or_reparse_ancestors(path).map_err(|_| identity_rejected())?;
        }
        if !broker_helper.is_file()
            || !launcher.is_file()
            || !codex_home.is_dir()
            || !isolation_root.is_dir()
            || !product_root.is_dir()
            || !cwd.is_dir()
            || !temp.is_dir()
            || cwd.parent() != Some(isolation_root.as_path())
            || temp.parent() != Some(isolation_root.as_path())
            || config_lock.parent() != Some(isolation_root.as_path())
            || crate::path_is_within(&isolation_root, &codex_home)
            || crate::path_is_within(&codex_home, &isolation_root)
            || crate::path_is_within(&isolation_root, &product_root)
            || crate::path_is_within(&product_root, &isolation_root)
        {
            return Err(identity_rejected());
        }
        if fs::read_dir(&cwd)
            .map_err(|_| identity_rejected())?
            .next()
            .is_some()
        {
            return Err(identity_rejected());
        }
        validate_broker_codex_home(&codex_home, &product_root).map_err(|_| identity_rejected())?;
        let lock_bytes = bounded_file_bytes(&config_lock, MAX_BROKER_WIRE_BYTES as u64)
            .map_err(|_| identity_rejected())?;
        if lock_bytes != CODEX_CONFIG_LOCK.as_bytes() {
            return Err(identity_rejected());
        }
        let config_lock_sha256 = sha256_bytes(&lock_bytes);
        let helper_sha256 = bounded_file_sha256(&broker_helper, MAX_CODEX_LAUNCHER_BYTES)
            .map_err(|_| identity_rejected())?;
        if helper_sha256 != config.broker_helper_sha256 {
            return Err(identity_rejected());
        }
        let child_environment = codex_child_environment(&launcher, &codex_home, &temp)
            .map_err(|_| identity_rejected())?;
        let child_environment_sha256 =
            digest_environment(&child_environment).map_err(|_| identity_rejected())?;
        Ok(Self {
            broker_helper,
            child_environment_sha256,
            codex_home,
            config_lock,
            config_lock_sha256,
            cwd,
            helper_sha256,
            isolation_root,
            launcher,
            model: config.model,
            product_root,
            temp,
        })
    }

    fn preflight_receipt_digest(
        &self,
        reviewed: &ReviewedCodexBundle,
    ) -> HermesAdapterResult<ContentDigest> {
        if reviewed.version() != CODEX_VERSION
            || reviewed.launcher() != self.launcher
            || reviewed.launcher_sha256() != CODEX_LAUNCHER_SHA256
            || reviewed.package_manifest_sha256() != CODEX_PACKAGE_MANIFEST_SHA256
            || self.model != "gpt-5.3-codex-spark"
        {
            return Err(identity("HERMES_CODEX_BUNDLE_IDENTITY_REJECTED"));
        }
        let fields = [
            reviewed.launcher_sha256().to_owned(),
            reviewed.package_manifest_sha256().to_owned(),
            reviewed.version().to_owned(),
            CODEX_SANDBOX_SETUP_SHA256.to_owned(),
            CODEX_COMMAND_RUNNER_SHA256.to_owned(),
            self.helper_sha256.clone(),
            self.config_lock_sha256.clone(),
            self.child_environment_sha256.clone(),
            path_text(&self.broker_helper)?,
            path_text(&self.codex_home)?,
            path_text(&self.config_lock)?,
            path_text(&self.cwd)?,
            path_text(&self.isolation_root)?,
            path_text(&self.launcher)?,
            path_text(&self.product_root)?,
            path_text(&self.temp)?,
            self.model.clone(),
        ];
        let mut sealed = Sha256::new();
        sealed.update(b"lattice.hermes.codex-broker-zero-model-preflight.v1\0");
        for field in fields {
            sealed.update((field.len() as u64).to_be_bytes());
            sealed.update(field.as_bytes());
        }
        ContentDigest::from_sha256(encode_digest(&sealed.finalize()))
            .map_err(|_| malformed_error("HERMES_CODEX_BROKER_PREFLIGHT_RECEIPT_REJECTED"))
    }

    fn command_plan(
        &self,
        reviewed: &ReviewedCodexBundle,
        deadline: Instant,
    ) -> HermesAdapterResult<crate::windows_job::WindowsJobCommandPlan> {
        if deadline <= Instant::now()
            || reviewed.version() != CODEX_VERSION
            || reviewed.launcher() != self.launcher
            || reviewed.launcher_sha256() != CODEX_LAUNCHER_SHA256
            || reviewed.package_manifest_sha256() != CODEX_PACKAGE_MANIFEST_SHA256
            || self.model != "gpt-5.3-codex-spark"
        {
            return Err(HermesAdapterError::new(
                HermesAdapterErrorKind::Identity,
                "HERMES_CODEX_PROXY_CONFIG_IDENTITY_REJECTED",
            ));
        }
        CodexProxyInvocation::parse(["app-server", "--listen", "stdio://", "--strict-config"])?;
        let environment = codex_child_environment(&self.launcher, &self.codex_home, &self.temp)?;
        if digest_environment(&environment)? != self.child_environment_sha256 {
            return Err(HermesAdapterError::new(
                HermesAdapterErrorKind::CrossBinding,
                "HERMES_CODEX_PROXY_FACTORY_BINDING_REJECTED",
            ));
        }
        let config_override = config_lock_override(&self.config_lock)?;
        Ok(crate::windows_job::WindowsJobCommandPlan {
            executable: self.launcher.clone(),
            arguments: [
                OsString::from("app-server"),
                OsString::from("--listen"),
                OsString::from("stdio://"),
                OsString::from("--strict-config"),
                OsString::from("-c"),
                OsString::from(config_override),
                OsString::from("-c"),
                OsString::from(CODEX_CONFIG_LOCK_RESOLUTION_OVERRIDE),
            ]
            .into_iter()
            .collect(),
            current_dir: self.cwd.clone(),
            environment,
            run_root: self.isolation_root.clone(),
            stdout_path: self.isolation_root.join("codex-proxy.stdout.unused"),
            stderr_path: self.isolation_root.join("codex-proxy.stderr.unused"),
            stdout_limit: MAX_CODEX_FRAME_BYTES as u64,
            stderr_limit: MAX_CODEX_PROXY_STDERR_BYTES,
            deadline,
            teardown_timeout: Duration::from_secs(3),
        })
    }

    fn reverify_open(&self) -> HermesAdapterResult<ReviewedCodexBundle> {
        let current = Self::from_config(CodexReflectionBrokerConfig {
            broker_helper: self.broker_helper.clone(),
            broker_helper_sha256: self.helper_sha256.clone(),
            codex_home: self.codex_home.clone(),
            isolation_root: self.isolation_root.clone(),
            launcher: self.launcher.clone(),
            model: self.model.clone(),
            product_root: self.product_root.clone(),
        })?;
        if current != *self {
            return Err(HermesAdapterError::new(
                HermesAdapterErrorKind::Identity,
                "HERMES_CODEX_PROXY_CONFIG_IDENTITY_REJECTED",
            ));
        }
        let reviewed = verify_official_codex_bundle(&self.launcher)?;
        if reviewed.version() != CODEX_VERSION
            || reviewed.launcher_sha256() != CODEX_LAUNCHER_SHA256
            || reviewed.package_manifest_sha256() != CODEX_PACKAGE_MANIFEST_SHA256
        {
            return Err(HermesAdapterError::new(
                HermesAdapterErrorKind::Identity,
                "HERMES_CODEX_BUNDLE_IDENTITY_REJECTED",
            ));
        }
        Ok(reviewed)
    }
}

#[cfg(windows)]
struct OfficialCodexProxyProvider {
    verified: VerifiedCodexProxyConfig,
    control: Arc<OwnedCodexProxyControl>,
}

#[cfg(windows)]
impl ProductionCodexProxyProvider for OfficialCodexProxyProvider {
    fn control(&self) -> Arc<dyn ProductionCodexProxyControl> {
        self.control.clone()
    }

    fn open(
        self: Box<Self>,
        absolute_deadline: Instant,
    ) -> HermesAdapterResult<ProductionCodexProxyDuplex> {
        if absolute_deadline <= Instant::now() {
            return Err(timeout("HERMES_CODEX_PROXY_DEADLINE_EXCEEDED"));
        }
        let Self { verified, control } = *self;
        control.ensure_open_allowed()?;
        let reviewed = verified.reverify_open()?;
        control.ensure_open_allowed()?;
        let plan = verified.command_plan(&reviewed, absolute_deadline)?;
        launch_owned_proxy(
            &plan,
            MAX_CODEX_PROXY_STDERR_BYTES,
            &control,
            || verified.reverify_open().map(|_| ()),
            |_| {},
        )
    }
}

#[cfg(windows)]
#[derive(Clone, Debug, Eq, PartialEq)]
struct BoundedCodexStderrEvidence {
    byte_count: u64,
    sha256: String,
    exceeded: bool,
}

#[cfg(windows)]
struct OwnedCodexProxyState {
    child: Option<crate::windows_job::WindowsJobChild>,
    stderr_evidence: Option<BoundedCodexStderrEvidence>,
    stderr_limit: u64,
    stderr_thread: Option<JoinHandle<Result<BoundedCodexStderrEvidence, ()>>>,
    terminal_failure: Option<HermesAdapterError>,
}

#[cfg(windows)]
impl OwnedCodexProxyState {
    fn poll_stderr(&mut self) -> HermesAdapterResult<()> {
        if self
            .stderr_thread
            .as_ref()
            .is_some_and(JoinHandle::is_finished)
        {
            self.join_stderr()?;
        }
        Ok(())
    }

    fn join_stderr(&mut self) -> HermesAdapterResult<()> {
        let Some(thread) = self.stderr_thread.take() else {
            let result = self.validate_stderr_evidence();
            if let Err(failure) = &result {
                self.terminal_failure = Some(failure.clone());
            }
            return result;
        };
        let evidence = match thread.join() {
            Ok(Ok(evidence)) => evidence,
            Ok(Err(())) => {
                let failure = HermesAdapterError::new(
                    HermesAdapterErrorKind::Transport,
                    "HERMES_CODEX_PROXY_STDERR_DRAIN_FAILED",
                );
                self.terminal_failure = Some(failure.clone());
                return Err(failure);
            }
            Err(_) => {
                let failure = HermesAdapterError::new(
                    HermesAdapterErrorKind::Ambiguous,
                    "HERMES_CODEX_PROXY_STDERR_DRAIN_AMBIGUOUS",
                );
                self.terminal_failure = Some(failure.clone());
                return Err(failure);
            }
        };
        self.stderr_evidence = Some(evidence);
        let result = self.validate_stderr_evidence();
        if let Err(failure) = &result {
            self.terminal_failure = Some(failure.clone());
        }
        result
    }

    fn validate_stderr_evidence(&self) -> HermesAdapterResult<()> {
        let Some(evidence) = &self.stderr_evidence else {
            return Ok(());
        };
        if !is_lowercase_sha256(&evidence.sha256) || evidence.byte_count > self.stderr_limit + 1 {
            return Err(HermesAdapterError::new(
                HermesAdapterErrorKind::Ambiguous,
                "HERMES_CODEX_PROXY_STDERR_EVIDENCE_AMBIGUOUS",
            ));
        }
        if evidence.exceeded {
            return Err(HermesAdapterError::new(
                HermesAdapterErrorKind::Malformed,
                "HERMES_CODEX_PROXY_STDERR_LIMIT_EXCEEDED",
            ));
        }
        Ok(())
    }

    fn terminate(&mut self) -> HermesAdapterResult<()> {
        let prior_failure = self.terminal_failure.clone();
        let termination = if let Some(child) = self.child.as_mut() {
            child.terminate()
        } else {
            Ok(())
        };
        let stderr = self.join_stderr();
        if let Err(failure) = termination {
            self.terminal_failure = Some(failure.clone());
            return Err(failure);
        }
        stderr?;
        prior_failure.map_or(Ok(()), Err)
    }
}

#[cfg(windows)]
struct OwnedCodexProxyControl {
    cancelled: AtomicBool,
    state: Mutex<OwnedCodexProxyState>,
}

#[cfg(windows)]
impl OwnedCodexProxyControl {
    fn new(stderr_limit: u64) -> Self {
        Self {
            cancelled: AtomicBool::new(false),
            state: Mutex::new(OwnedCodexProxyState {
                child: None,
                stderr_evidence: None,
                stderr_limit,
                stderr_thread: None,
                terminal_failure: None,
            }),
        }
    }

    fn ensure_open_allowed(&self) -> HermesAdapterResult<()> {
        if self.cancelled.load(Ordering::Acquire) {
            return Err(HermesAdapterError::new(
                HermesAdapterErrorKind::Cancelled,
                "HERMES_CODEX_PROXY_OPEN_CANCELLED",
            ));
        }
        Ok(())
    }

    fn ensure_unbound(&self) -> HermesAdapterResult<()> {
        self.ensure_open_allowed()?;
        let state = self.state.lock().map_err(|_| {
            HermesAdapterError::new(
                HermesAdapterErrorKind::Ambiguous,
                "HERMES_CODEX_PROXY_CONTROL_STATE_UNKNOWN",
            )
        })?;
        if state.child.is_some() {
            return Err(HermesAdapterError::new(
                HermesAdapterErrorKind::Ambiguous,
                "HERMES_CODEX_PROXY_PROCESS_ALREADY_BOUND",
            ));
        }
        Ok(())
    }

    fn bind_child(
        &self,
        mut child: crate::windows_job::WindowsJobChild,
    ) -> HermesAdapterResult<()> {
        let Ok(mut state) = self.state.lock() else {
            child.terminate()?;
            return Err(HermesAdapterError::new(
                HermesAdapterErrorKind::Ambiguous,
                "HERMES_CODEX_PROXY_CONTROL_STATE_UNKNOWN",
            ));
        };
        if state.child.is_some() {
            child.terminate()?;
            return Err(HermesAdapterError::new(
                HermesAdapterErrorKind::Ambiguous,
                "HERMES_CODEX_PROXY_PROCESS_ALREADY_BOUND",
            ));
        }
        if self.cancelled.load(Ordering::Acquire) {
            child.terminate()?;
            return Err(HermesAdapterError::new(
                HermesAdapterErrorKind::Cancelled,
                "HERMES_CODEX_PROXY_OPEN_CANCELLED",
            ));
        }
        state.child = Some(child);
        Ok(())
    }

    fn start_stdio(&self) -> HermesAdapterResult<(Box<dyn Read + Send>, Box<dyn Write + Send>)> {
        let mut state = self.state.lock().map_err(|_| {
            HermesAdapterError::new(
                HermesAdapterErrorKind::Ambiguous,
                "HERMES_CODEX_PROXY_CONTROL_STATE_UNKNOWN",
            )
        })?;
        if self.cancelled.load(Ordering::Acquire) {
            return Err(HermesAdapterError::new(
                HermesAdapterErrorKind::Cancelled,
                "HERMES_CODEX_PROXY_OPEN_CANCELLED",
            ));
        }
        if state.stderr_thread.is_some() || state.stderr_evidence.is_some() {
            return Err(HermesAdapterError::new(
                HermesAdapterErrorKind::Ambiguous,
                "HERMES_CODEX_PROXY_STDIO_ALREADY_BOUND",
            ));
        }
        let stderr_limit = state.stderr_limit;
        let child = state.child.as_mut().ok_or_else(|| {
            HermesAdapterError::new(
                HermesAdapterErrorKind::Ambiguous,
                "HERMES_CODEX_PROXY_PROCESS_NOT_STARTED",
            )
        })?;
        let writer = child.take_stdin_writer()?;
        let reader = child.take_stdout_reader()?;
        let stderr = child.take_stderr_reader()?;
        let stderr_thread = thread::Builder::new()
            .name("lattice-codex-stderr".to_owned())
            .spawn(move || drain_bounded_codex_stderr(stderr, stderr_limit))
            .map_err(|_| spawn("HERMES_CODEX_PROXY_STDERR_THREAD_SPAWN_FAILED"))?;
        state.stderr_thread = Some(stderr_thread);
        Ok((Box::new(reader), Box::new(writer)))
    }
}

#[cfg(windows)]
impl ProductionCodexProxyControl for OwnedCodexProxyControl {
    fn ensure_running(&self) -> HermesAdapterResult<()> {
        self.ensure_open_allowed()?;
        let mut state = self.state.lock().map_err(|_| {
            HermesAdapterError::new(
                HermesAdapterErrorKind::Ambiguous,
                "HERMES_CODEX_PROXY_CONTROL_STATE_UNKNOWN",
            )
        })?;
        if state.terminal_failure.is_some() {
            return state.terminate();
        }
        if let Err(failure) = state.poll_stderr() {
            state.terminate()?;
            return Err(failure);
        }
        let running = state.child.as_mut().ok_or_else(|| {
            HermesAdapterError::new(
                HermesAdapterErrorKind::Ambiguous,
                "HERMES_CODEX_PROXY_PROCESS_NOT_STARTED",
            )
        })?;
        match running.ensure_running() {
            Ok(()) => state.poll_stderr(),
            Err(failure) => {
                let _ = state.join_stderr();
                Err(failure)
            }
        }
    }

    fn terminate(&self) -> HermesAdapterResult<()> {
        self.cancelled.store(true, Ordering::Release);
        self.state
            .lock()
            .map_err(|_| {
                HermesAdapterError::new(
                    HermesAdapterErrorKind::Ambiguous,
                    "HERMES_CODEX_PROXY_CONTROL_STATE_UNKNOWN",
                )
            })?
            .terminate()
    }
}

#[cfg(windows)]
impl Drop for OwnedCodexProxyControl {
    fn drop(&mut self) {
        self.cancelled.store(true, Ordering::Release);
        if let Ok(state) = self.state.get_mut() {
            let _ = state.terminate();
        }
    }
}

#[cfg(windows)]
fn launch_owned_proxy<F, G>(
    plan: &crate::windows_job::WindowsJobCommandPlan,
    stderr_limit: u64,
    control: &Arc<OwnedCodexProxyControl>,
    post_spawn_identity_check: F,
    observe_process_id: G,
) -> HermesAdapterResult<ProductionCodexProxyDuplex>
where
    F: FnOnce() -> HermesAdapterResult<()>,
    G: FnOnce(u32),
{
    if stderr_limit == 0 || stderr_limit > MAX_CODEX_PROXY_STDERR_BYTES {
        return Err(configuration("HERMES_CODEX_PROXY_STDERR_LIMIT_REJECTED"));
    }
    control.ensure_unbound()?;
    let child = crate::windows_job::spawn_duplex(plan)?;
    let process_id = child.process_id();
    control.bind_child(child)?;
    observe_process_id(process_id);
    let (reader, writer) = match control.start_stdio() {
        Ok(streams) => streams,
        Err(failure) => {
            control.terminate()?;
            return Err(failure);
        }
    };
    if let Err(failure) = post_spawn_identity_check() {
        control.terminate()?;
        return Err(failure);
    }
    control.ensure_running()?;
    Ok(ProductionCodexProxyDuplex::new(reader, writer))
}

#[cfg(windows)]
fn drain_bounded_codex_stderr(
    mut stderr: File,
    limit: u64,
) -> Result<BoundedCodexStderrEvidence, ()> {
    let mut digest = Sha256::new();
    let mut byte_count = 0_u64;
    let mut buffer = [0_u8; 4096];
    loop {
        let remaining = limit
            .checked_add(1)
            .and_then(|bound| bound.checked_sub(byte_count))
            .ok_or(())?;
        if remaining == 0 {
            return Ok(BoundedCodexStderrEvidence {
                byte_count,
                sha256: encode_digest(&digest.finalize()),
                exceeded: true,
            });
        }
        let capacity = usize::try_from(remaining.min(buffer.len() as u64)).map_err(|_| ())?;
        let read = stderr.read(&mut buffer[..capacity]).map_err(|_| ())?;
        if read == 0 {
            return Ok(BoundedCodexStderrEvidence {
                byte_count,
                sha256: encode_digest(&digest.finalize()),
                exceeded: false,
            });
        }
        digest.update(&buffer[..read]);
        byte_count = byte_count
            .checked_add(u64::try_from(read).map_err(|_| ())?)
            .ok_or(())?;
    }
}

#[cfg(all(test, windows))]
struct FixtureCodexProxyProvider {
    control: Arc<OwnedCodexProxyControl>,
    executable_sha256: String,
    plan: crate::windows_job::WindowsJobCommandPlan,
    process_id: std::sync::Arc<std::sync::atomic::AtomicU32>,
    stderr_limit: u64,
}

#[cfg(all(test, windows))]
impl ProductionCodexProxyProvider for FixtureCodexProxyProvider {
    fn control(&self) -> Arc<dyn ProductionCodexProxyControl> {
        self.control.clone()
    }

    fn open(
        self: Box<Self>,
        absolute_deadline: Instant,
    ) -> HermesAdapterResult<ProductionCodexProxyDuplex> {
        if absolute_deadline <= Instant::now() {
            return Err(timeout("HERMES_CODEX_PROXY_DEADLINE_EXCEEDED"));
        }
        let Self {
            control,
            executable_sha256,
            mut plan,
            process_id,
            stderr_limit,
        } = *self;
        let executable = fs::canonicalize(&plan.executable)
            .map_err(|_| identity("HERMES_CODEX_PROXY_FIXTURE_IDENTITY_REJECTED"))?;
        if bounded_file_sha256(&executable, MAX_CODEX_LAUNCHER_BYTES)? != executable_sha256 {
            return Err(identity("HERMES_CODEX_PROXY_FIXTURE_IDENTITY_REJECTED"));
        }
        plan.executable.clone_from(&executable);
        plan.deadline = absolute_deadline;
        launch_owned_proxy(
            &plan,
            stderr_limit,
            &control,
            move || {
                if bounded_file_sha256(&executable, MAX_CODEX_LAUNCHER_BYTES)? == executable_sha256
                {
                    Ok(())
                } else {
                    Err(identity("HERMES_CODEX_PROXY_FIXTURE_IDENTITY_REJECTED"))
                }
            },
            move |observed| {
                process_id.store(observed, std::sync::atomic::Ordering::Release);
            },
        )
    }
}

#[cfg(windows)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexBrokerPreflightReceipt {
    child_environment_sha256: String,
    config_lock_sha256: String,
    helper_sha256: String,
    launcher_sha256: String,
    receipt_digest: ContentDigest,
    #[cfg(test)]
    test_only: bool,
}

#[cfg(windows)]
impl CodexBrokerPreflightReceipt {
    #[cfg(test)]
    fn test_only(
        child_environment_sha256: String,
        config_lock_sha256: String,
        helper_sha256: String,
        launcher_sha256: String,
    ) -> Self {
        Self {
            child_environment_sha256,
            config_lock_sha256,
            helper_sha256,
            launcher_sha256,
            receipt_digest: ContentDigest::from_sha256("b".repeat(64))
                .expect("test-only receipt digest"),
            test_only: true,
        }
    }

    /// Digest of the exact zero-model configuration and path binding.
    #[must_use]
    pub const fn receipt_digest(&self) -> &ContentDigest {
        &self.receipt_digest
    }

    /// Digest of the exact strict no-tools config lock.
    #[must_use]
    pub fn config_lock_sha256(&self) -> &str {
        &self.config_lock_sha256
    }

    /// Digest of the complete scrubbed child environment.
    #[must_use]
    pub fn child_environment_sha256(&self) -> &str {
        &self.child_environment_sha256
    }

    /// Digest of the Job-contained broker helper executable.
    #[must_use]
    pub fn helper_sha256(&self) -> &str {
        &self.helper_sha256
    }

    /// Digest of the official Codex launcher.
    #[must_use]
    pub fn launcher_sha256(&self) -> &str {
        &self.launcher_sha256
    }

    pub(crate) fn validate_for_containment(&self) -> HermesAdapterResult<()> {
        if self.launcher_sha256 != CODEX_LAUNCHER_SHA256
            || self.config_lock_sha256 != sha256_bytes(CODEX_CONFIG_LOCK.as_bytes())
            || !is_lowercase_sha256(&self.child_environment_sha256)
            || !is_lowercase_sha256(&self.helper_sha256)
            || self.receipt_digest.as_str().len() != 64
        {
            return Err(HermesAdapterError::new(
                HermesAdapterErrorKind::CrossBinding,
                "HERMES_CODEX_BROKER_PREFLIGHT_BINDING_REJECTED",
            ));
        }
        Ok(())
    }
}

#[cfg(windows)]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct CodexBrokerHelperRequest {
    codex_home: String,
    config_lock: String,
    cwd: String,
    deadline_millis: u64,
    launcher: String,
    model: String,
    nonce: String,
    schema: String,
    temp: String,
}

#[cfg(windows)]
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct CodexBrokerCandidate {
    agent_message_count: u64,
    child_environment_sha256: String,
    environment_connection_count: u64,
    forbidden_event_count: u64,
    marker_existed_before: bool,
    marker_exists_after: bool,
    nonce: String,
    output: String,
    schema: String,
    stderr_sha256: String,
    terminal_status: String,
    transcript_sha256: String,
    tree_file_count_after: u64,
    tree_file_count_before: u64,
    tree_sha256_after: String,
    tree_sha256_before: String,
}

/// Executes the private broker-helper mode. Direct execution is fail-closed:
/// the helper must already belong to the parent's kill-on-close Job Object.
pub(crate) fn run_codex_reflection_broker_helper() -> i32 {
    #[cfg(not(windows))]
    {
        64
    }
    #[cfg(windows)]
    {
        match run_broker_helper_inner() {
            Ok(candidate) => match emit_broker_candidate(&candidate) {
                Ok(()) => 0,
                Err(code) => {
                    emit_helper_failure(code);
                    code
                }
            },
            Err(code) => {
                emit_helper_failure(code);
                code
            }
        }
    }
}

#[cfg(windows)]
#[allow(clippy::too_many_lines)]
fn run_broker_helper_inner() -> Result<CodexBrokerCandidate, i32> {
    if !crate::windows_job::current_process_is_in_job() {
        return Err(64);
    }
    let arguments = std::env::args_os().skip(1).collect::<Vec<_>>();
    let [mode, request_path] = arguments.as_slice() else {
        return Err(64);
    };
    if mode != "broker-helper" {
        return Err(64);
    }
    let request_bytes = read_bounded_helper_file(Path::new(request_path), MAX_BROKER_WIRE_BYTES)
        .map_err(|()| 64)?;
    let request: CodexBrokerHelperRequest =
        serde_json::from_slice(&request_bytes).map_err(|_| 64)?;
    if serde_json::to_vec(&request).map_err(|_| 64)? != request_bytes
        || request.schema != BROKER_HELPER_SCHEMA
        || request.model != "gpt-5.3-codex-spark"
        || !is_lowercase_sha256(&request.nonce)
        || request.deadline_millis == 0
        || request.deadline_millis > 300_000
    {
        return Err(64);
    }
    let deadline = Instant::now()
        .checked_add(Duration::from_millis(request.deadline_millis))
        .ok_or(69)?;
    let launcher = canonical_request_path(&request.launcher, true).map_err(|_| 65)?;
    let reviewed = verify_official_codex_bundle(&launcher).map_err(|_| 65)?;
    let codex_home = canonical_request_path(&request.codex_home, false).map_err(|_| 66)?;
    let config_lock = canonical_request_path(&request.config_lock, true).map_err(|_| 66)?;
    let cwd = canonical_request_path(&request.cwd, false).map_err(|_| 66)?;
    let temp = canonical_request_path(&request.temp, false).map_err(|_| 66)?;
    let request_path = fs::canonicalize(request_path).map_err(|_| 64)?;
    let run_root = request_path.parent().ok_or(64)?;
    if config_lock.parent() != Some(run_root)
        || cwd.parent() != Some(run_root)
        || temp.parent() != Some(run_root)
        || codex_home == run_root
        || crate::path_is_within(run_root, &codex_home)
        || crate::path_is_within(&codex_home, run_root)
    {
        return Err(66);
    }
    let lock_bytes =
        read_bounded_helper_file(&config_lock, MAX_BROKER_WIRE_BYTES).map_err(|()| 66)?;
    if lock_bytes != CODEX_CONFIG_LOCK.as_bytes() {
        return Err(66);
    }
    match fs::symlink_metadata(codex_home.join("environments.toml")) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        _ => return Err(66),
    }
    ensure_before_helper_deadline(deadline)?;
    let (tree_sha256_before, tree_file_count_before) = empty_tree_identity(&cwd)?;
    let marker = cwd.join(format!(".lattice-hermes-no-tools-{}", request.nonce));
    let marker_existed_before = marker.exists();
    let child_environment =
        codex_child_environment(reviewed.launcher(), &codex_home, &temp).map_err(|_| 66)?;
    let child_environment_sha256 = digest_environment(&child_environment).map_err(|_| 66)?;
    let config_override = config_lock_override(&config_lock).map_err(|_| 66)?;
    CodexProxyInvocation::parse(["app-server", "--listen", "stdio://", "--strict-config"])
        .map_err(|_| 66)?;
    let mut command = Command::new(reviewed.launcher());
    command
        .args([
            "app-server",
            "--listen",
            "stdio://",
            "--strict-config",
            "-c",
            &config_override,
            "-c",
            CODEX_CONFIG_LOCK_RESOLUTION_OVERRIDE,
        ])
        .current_dir(&cwd)
        .env_clear()
        .envs(&child_environment)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    ensure_before_helper_deadline(deadline)?;
    let child = command.spawn().map_err(|_| 67)?;
    let mut guard = BrokerChildGuard::new(child);
    if bounded_file_sha256(reviewed.launcher(), MAX_CODEX_LAUNCHER_BYTES).map_err(|_| 65)?
        != reviewed.launcher_sha256()
    {
        return Err(65);
    }
    let mut stdin = guard.child.stdin.take().ok_or(67)?;
    let stdout = guard.child.stdout.take().ok_or(67)?;
    let stderr = guard.child.stderr.take().ok_or(67)?;
    let receiver = start_codex_stdout_reader(stdout);
    let stderr_reader = start_codex_stderr_reader(stderr);
    let mut transcript = Sha256::new();
    transcript.update(b"lattice.hermes.codex-broker-transcript.v1\0");
    let plan =
        CodexNoMarkerCanaryPlan::new(cwd.clone(), request.nonce.clone(), request.model.clone())
            .map_err(|_| 66)?;
    let mut protocol =
        CodexBrokerProtocol::new(codex_home.clone(), cwd.clone(), request.model.clone())?;

    protocol.mark_request_sent(CodexBrokerRequest::Initialize)?;
    send_codex_json(&mut stdin, &plan.initialize_request(), &mut transcript).map_err(|_| 83)?;
    while !protocol.responses_seen[CodexBrokerRequest::Initialize.index()] {
        let frame = receive_codex_frame(&receiver, &mut transcript, deadline)?;
        ingest_codex_broker_frame(&mut protocol, &frame, &mut stdin, &mut transcript)?;
    }
    send_codex_json(
        &mut stdin,
        &plan.initialized_notification(),
        &mut transcript,
    )
    .map_err(|_| 84)?;
    protocol.mark_request_sent(CodexBrokerRequest::ThreadStart)?;
    send_codex_json(&mut stdin, &plan.thread_start_request(), &mut transcript).map_err(|_| 84)?;
    while protocol.thread_id.is_none() {
        let frame = receive_codex_frame(&receiver, &mut transcript, deadline)?;
        ingest_codex_broker_frame(&mut protocol, &frame, &mut stdin, &mut transcript)?;
    }
    let thread_id = protocol.thread_id.clone().ok_or(72)?;
    protocol.mark_request_sent(CodexBrokerRequest::TurnStart)?;
    send_codex_json(
        &mut stdin,
        &plan.turn_start_request(&thread_id),
        &mut transcript,
    )
    .map_err(|_| 85)?;
    let terminal = loop {
        let frame = receive_codex_frame(&receiver, &mut transcript, deadline)?;
        if let Some(terminal) =
            ingest_codex_broker_frame(&mut protocol, &frame, &mut stdin, &mut transcript)?
        {
            break terminal;
        }
    };
    guard.stop().map_err(|()| 67)?;
    drop(stdin);
    let (stderr_sha256, stderr_bytes, stderr_overflow) = stderr_reader.join().map_err(|_| 67)?;
    if stderr_overflow || stderr_bytes > MAX_CODEX_STDERR_BYTES {
        return Err(78);
    }
    let (tree_sha256_after, tree_file_count_after) = empty_tree_identity(&cwd)?;
    let marker_exists_after = marker.exists();
    Ok(CodexBrokerCandidate {
        agent_message_count: terminal.agent_message_count,
        child_environment_sha256,
        environment_connection_count: 0,
        forbidden_event_count: 0,
        marker_existed_before,
        marker_exists_after,
        nonce: request.nonce,
        output: terminal.output,
        schema: BROKER_CANDIDATE_SCHEMA.to_owned(),
        stderr_sha256,
        terminal_status: terminal.status,
        transcript_sha256: encode_digest(&transcript.finalize()),
        tree_file_count_after,
        tree_file_count_before,
        tree_sha256_after,
        tree_sha256_before,
    })
}

#[cfg(windows)]
fn emit_broker_candidate(candidate: &CodexBrokerCandidate) -> Result<(), i32> {
    let encoded = serde_json::to_vec(candidate).map_err(|_| 70)?;
    if encoded.len() > MAX_BROKER_WIRE_BYTES {
        return Err(70);
    }
    let mut output = std::io::stdout().lock();
    output.write_all(BROKER_HELPER_MAGIC).map_err(|_| 70)?;
    output
        .write_all(&(encoded.len() as u64).to_be_bytes())
        .map_err(|_| 70)?;
    output.write_all(&encoded).map_err(|_| 70)?;
    output.flush().map_err(|_| 70)
}

#[cfg(windows)]
fn emit_helper_failure(code: i32) {
    let _ = writeln!(
        std::io::stderr().lock(),
        "HERMES_CODEX_BROKER_HELPER_FAIL:{code}"
    );
}

#[cfg(windows)]
struct BrokerChildGuard {
    child: Child,
    stopped: bool,
}

#[cfg(windows)]
impl BrokerChildGuard {
    fn new(child: Child) -> Self {
        Self {
            child,
            stopped: false,
        }
    }

    fn stop(&mut self) -> Result<(), ()> {
        if self.stopped {
            return Ok(());
        }
        let _ = self.child.kill();
        self.child.wait().map_err(|_| ())?;
        self.stopped = true;
        Ok(())
    }
}

#[cfg(windows)]
impl Drop for BrokerChildGuard {
    fn drop(&mut self) {
        if !self.stopped {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

#[cfg(windows)]
enum CodexReaderEvent {
    Line(Vec<u8>),
    Eof,
    Failed,
    TooLarge,
}

#[cfg(windows)]
struct ReceivedCodexFrame {
    kind: CodexAppServerFrameKind,
    value: Value,
}

#[cfg(windows)]
fn start_codex_stdout_reader(stdout: std::process::ChildStdout) -> Receiver<CodexReaderEvent> {
    let (sender, receiver) = mpsc::channel();
    thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            let mut line = Vec::new();
            let read = reader
                .by_ref()
                .take((MAX_CODEX_FRAME_BYTES + 2) as u64)
                .read_until(b'\n', &mut line);
            match read {
                Ok(0) => {
                    let _ = sender.send(CodexReaderEvent::Eof);
                    return;
                }
                Ok(_) if line.len() > MAX_CODEX_FRAME_BYTES + 1 || !line.ends_with(b"\n") => {
                    let _ = sender.send(CodexReaderEvent::TooLarge);
                    return;
                }
                Ok(_) => {
                    line.pop();
                    if line.last() == Some(&b'\r') {
                        line.pop();
                    }
                    if line.is_empty() || line.len() > MAX_CODEX_FRAME_BYTES {
                        let _ = sender.send(CodexReaderEvent::TooLarge);
                        return;
                    }
                    if sender.send(CodexReaderEvent::Line(line)).is_err() {
                        return;
                    }
                }
                Err(_) => {
                    let _ = sender.send(CodexReaderEvent::Failed);
                    return;
                }
            }
        }
    });
    receiver
}

#[cfg(windows)]
fn start_codex_stderr_reader(
    mut stderr: std::process::ChildStderr,
) -> JoinHandle<(String, u64, bool)> {
    thread::spawn(move || {
        let mut digest = Sha256::new();
        let mut count = 0_u64;
        let mut overflow = false;
        let mut buffer = [0_u8; 8192];
        loop {
            match stderr.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    count = count.saturating_add(read as u64);
                    overflow |= count > MAX_CODEX_STDERR_BYTES;
                    digest.update(&buffer[..read]);
                }
                Err(_) => {
                    overflow = true;
                    break;
                }
            }
        }
        (encode_digest(&digest.finalize()), count, overflow)
    })
}

#[cfg(windows)]
fn send_codex_json(
    stdin: &mut ChildStdin,
    value: &Value,
    transcript: &mut Sha256,
) -> std::io::Result<()> {
    let encoded = serde_json::to_vec(value)?;
    if encoded.len() > MAX_CODEX_FRAME_BYTES {
        return Err(std::io::Error::other("bounded request exceeded"));
    }
    transcript.update(b"C\0");
    transcript.update((encoded.len() as u64).to_be_bytes());
    transcript.update(&encoded);
    stdin.write_all(&encoded)?;
    stdin.write_all(b"\n")?;
    stdin.flush()
}

#[cfg(windows)]
fn receive_codex_frame(
    receiver: &Receiver<CodexReaderEvent>,
    transcript: &mut Sha256,
    deadline: Instant,
) -> Result<ReceivedCodexFrame, i32> {
    let remaining = deadline.checked_duration_since(Instant::now()).ok_or(69)?;
    let line = match receiver.recv_timeout(remaining) {
        Ok(CodexReaderEvent::Line(line)) => line,
        Ok(CodexReaderEvent::Eof) => return Err(79),
        Ok(CodexReaderEvent::Failed) | Err(RecvTimeoutError::Disconnected) => return Err(80),
        Ok(CodexReaderEvent::TooLarge) => return Err(81),
        Err(RecvTimeoutError::Timeout) => return Err(69),
    };
    transcript.update(b"S\0");
    transcript.update((line.len() as u64).to_be_bytes());
    transcript.update(&line);
    let value = parse_bounded_codex_json(&line).map_err(|_| 82)?;
    let kind = classify_codex_app_server_envelope(&value).map_err(|_| 75)?;
    Ok(ReceivedCodexFrame { kind, value })
}

#[cfg(windows)]
fn ingest_codex_broker_frame(
    protocol: &mut CodexBrokerProtocol,
    frame: &ReceivedCodexFrame,
    stdin: &mut ChildStdin,
    transcript: &mut Sha256,
) -> Result<Option<CodexBrokerTerminal>, i32> {
    if let CodexAppServerFrameKind::ServerRequest { id, .. } = &frame.kind {
        deny_and_interrupt(
            stdin,
            Some(*id),
            protocol.thread_id.as_deref(),
            protocol.active_turn_id(),
            transcript,
        );
        return Err(74);
    }
    protocol.ingest_frame(frame)
}

#[cfg(windows)]
fn deny_and_interrupt(
    stdin: &mut ChildStdin,
    request_id: Option<i64>,
    thread_id: Option<&str>,
    turn_id: Option<&str>,
    transcript: &mut Sha256,
) {
    if let Some(id) = request_id {
        let denied = serde_json::json!({
            "id": id,
            "error": {"code": -32601, "message": "LATTICE_BROKER_DENIED"}
        });
        let _ = send_codex_json(stdin, &denied, transcript);
    }
    if let (Some(thread_id), Some(turn_id)) = (
        thread_id.filter(|value| !value.is_empty()),
        turn_id.filter(|value| !value.is_empty()),
    ) {
        let interrupt = serde_json::json!({
            "id": 3,
            "method": "turn/interrupt",
            "params": {"threadId": thread_id, "turnId": turn_id}
        });
        let _ = send_codex_json(stdin, &interrupt, transcript);
    }
}

#[cfg(windows)]
fn validate_initialize_response(value: &Value, codex_home: &Path) -> Result<(), i32> {
    let result = success_result(value, 0)?;
    require_exact_keys(
        result,
        &["codexHome", "platformFamily", "platformOs", "userAgent"],
    )?;
    if result.get("platformFamily").and_then(Value::as_str) != Some("windows")
        || result.get("platformOs").and_then(Value::as_str) != Some("windows")
        || result.get("userAgent").and_then(Value::as_str) != Some("codex_cli_rs/0.146.0")
    {
        return Err(68);
    }
    let observed_home = result.get("codexHome").and_then(Value::as_str).ok_or(68)?;
    if !same_canonical_path(observed_home, codex_home) {
        return Err(68);
    }
    Ok(())
}

#[cfg(windows)]
fn validate_thread_start_response(value: &Value, cwd: &Path, model: &str) -> Result<String, i32> {
    let result = success_result(value, 1)?;
    crate::require_only_keys(
        result,
        &[
            "activePermissionProfile",
            "serviceTier",
            "approvalPolicy",
            "approvalsReviewer",
            "cwd",
            "instructionSources",
            "model",
            "modelProvider",
            "multiAgentMode",
            "runtimeWorkspaceRoots",
            "sandbox",
            "reasoningEffort",
            "thread",
        ],
        "HERMES_CODEX_BROKER_FATAL_FRAME",
    )
    .map_err(|_| 68)?;
    for required in [
        "activePermissionProfile",
        "approvalPolicy",
        "approvalsReviewer",
        "cwd",
        "instructionSources",
        "model",
        "modelProvider",
        "multiAgentMode",
        "reasoningEffort",
        "runtimeWorkspaceRoots",
        "sandbox",
        "serviceTier",
        "thread",
    ] {
        if !result.contains_key(required) {
            return Err(68);
        }
    }
    if result.get("approvalPolicy").and_then(Value::as_str) != Some("never")
        || result.get("approvalsReviewer").and_then(Value::as_str) != Some("user")
        || result.get("model").and_then(Value::as_str) != Some(model)
        || result.get("modelProvider").and_then(Value::as_str) != Some("openai")
        || result.get("reasoningEffort").and_then(Value::as_str) != Some("low")
        || result.get("multiAgentMode").and_then(Value::as_str) != Some("explicitRequestOnly")
        || !result
            .get("activePermissionProfile")
            .is_some_and(Value::is_null)
        || !result.get("serviceTier").is_some_and(Value::is_null)
        || !result
            .get("runtimeWorkspaceRoots")
            .and_then(Value::as_array)
            .is_some_and(Vec::is_empty)
        || !result
            .get("cwd")
            .and_then(Value::as_str)
            .is_some_and(|value| same_canonical_path(value, cwd))
        || result
            .get("instructionSources")
            .and_then(Value::as_array)
            .is_none_or(|sources| !sources.is_empty())
    {
        return Err(68);
    }
    validate_read_only_sandbox(result.get("sandbox").ok_or(68)?)?;
    let thread = result.get("thread").and_then(Value::as_object).ok_or(68)?;
    validate_thread_object(thread, cwd)
}

#[cfg(windows)]
#[allow(clippy::too_many_lines)]
fn validate_thread_object(thread: &Map<String, Value>, cwd: &Path) -> Result<String, i32> {
    crate::require_only_keys(
        thread,
        &[
            "agentNickname",
            "agentRole",
            "canAcceptDirectInput",
            "extra",
            "updatedAt",
            "cliVersion",
            "createdAt",
            "cwd",
            "ephemeral",
            "threadSource",
            "forkedFromId",
            "gitInfo",
            "historyMode",
            "turns",
            "id",
            "isPinned",
            "modelProvider",
            "name",
            "parentThreadId",
            "path",
            "preview",
            "recencyAt",
            "sessionId",
            "source",
            "status",
        ],
        "HERMES_CODEX_BROKER_FATAL_FRAME",
    )
    .map_err(|_| 68)?;
    for required in [
        "agentNickname",
        "agentRole",
        "canAcceptDirectInput",
        "cliVersion",
        "createdAt",
        "cwd",
        "ephemeral",
        "extra",
        "forkedFromId",
        "gitInfo",
        "historyMode",
        "id",
        "isPinned",
        "modelProvider",
        "name",
        "parentThreadId",
        "path",
        "preview",
        "recencyAt",
        "sessionId",
        "source",
        "status",
        "threadSource",
        "turns",
        "updatedAt",
    ] {
        if !thread.contains_key(required) {
            return Err(68);
        }
    }
    if thread.get("cliVersion").and_then(Value::as_str) != Some("0.146.0")
        || thread.get("ephemeral").and_then(Value::as_bool) != Some(true)
        || thread.get("isPinned").and_then(Value::as_bool) != Some(false)
        || thread.get("canAcceptDirectInput").and_then(Value::as_bool) != Some(true)
        || thread.get("historyMode").and_then(Value::as_str) != Some("legacy")
        || !thread
            .get("cwd")
            .and_then(Value::as_str)
            .is_some_and(|value| same_canonical_path(value, cwd))
        || thread
            .get("turns")
            .and_then(Value::as_array)
            .is_none_or(|turns| !turns.is_empty())
        || thread.get("modelProvider").and_then(Value::as_str) != Some("openai")
        || thread.get("preview").and_then(Value::as_str) != Some("")
        || !thread
            .get("sessionId")
            .and_then(Value::as_str)
            .is_some_and(|value| !value.is_empty() && value.len() <= 4_096)
        || thread.get("id") != thread.get("sessionId")
        || thread.get("createdAt").and_then(Value::as_i64).is_none()
        || thread.get("updatedAt").and_then(Value::as_i64).is_none()
        || thread.get("recencyAt").and_then(Value::as_i64).is_none()
        || [
            "agentNickname",
            "agentRole",
            "extra",
            "forkedFromId",
            "gitInfo",
            "name",
            "parentThreadId",
            "path",
            "threadSource",
        ]
        .iter()
        .any(|field| !thread.get(*field).is_some_and(Value::is_null))
        || !validate_session_source(thread.get("source").ok_or(68)?)
        || !validate_thread_status(thread.get("status").ok_or(68)?)
        || thread
            .get("status")
            .and_then(Value::as_object)
            .and_then(|status| status.get("type"))
            .and_then(Value::as_str)
            != Some("idle")
    {
        return Err(68);
    }
    required_nonempty_string(thread, "id")
}

#[cfg(windows)]
fn validate_session_source(value: &Value) -> bool {
    if matches!(
        value.as_str(),
        Some("cli" | "vscode" | "exec" | "appServer" | "unknown")
    ) {
        return true;
    }
    let Some(object) = value.as_object() else {
        return false;
    };
    (object.len() == 1
        && object
            .get("custom")
            .and_then(Value::as_str)
            .is_some_and(|source| !source.is_empty() && source.len() <= 256))
        || (object.len() == 1 && object.get("subAgent").is_some_and(Value::is_object))
}

#[cfg(windows)]
fn validate_thread_status(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    match object.get("type").and_then(Value::as_str) {
        Some("notLoaded" | "idle" | "systemError") => object.len() == 1,
        Some("active") => {
            object.len() == 2
                && object
                    .get("activeFlags")
                    .and_then(Value::as_array)
                    .is_some_and(|flags| {
                        flags.len() <= 2
                            && flags.iter().all(|flag| {
                                matches!(
                                    flag.as_str(),
                                    Some("waitingOnApproval" | "waitingOnUserInput")
                                )
                            })
                    })
        }
        _ => false,
    }
}

#[cfg(windows)]
fn validate_turn_start_response(value: &Value) -> Result<String, i32> {
    let result = success_result(value, 2)?;
    require_exact_keys(result, &["turn"])?;
    let turn = result.get("turn").and_then(Value::as_object).ok_or(68)?;
    validate_turn_object(turn)?;
    required_nonempty_string(turn, "id")
}

#[cfg(windows)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CodexBrokerTerminal {
    agent_message_count: u64,
    output: String,
    status: String,
}

impl CodexBrokerTerminal {
    #[cfg(test)]
    pub(crate) const fn agent_message_count(&self) -> u64 {
        self.agent_message_count
    }

    #[cfg(test)]
    pub(crate) fn output(&self) -> &str {
        &self.output
    }

    #[cfg(test)]
    pub(crate) fn status(&self) -> &str {
        &self.status
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CodexBrokerRequest {
    Initialize,
    ThreadStart,
    TurnStart,
}

impl CodexBrokerRequest {
    const fn index(self) -> usize {
        match self {
            Self::Initialize => 0,
            Self::ThreadStart => 1,
            Self::TurnStart => 2,
        }
    }

    const fn error_code(self) -> i32 {
        match self {
            Self::Initialize => 71,
            Self::ThreadStart => 72,
            Self::TurnStart => 73,
        }
    }
}

pub(crate) struct CodexBrokerProtocol {
    codex_home: PathBuf,
    cwd: PathBuf,
    model: String,
    requests_sent: [bool; 3],
    responses_seen: [bool; 3],
    thread_id: Option<String>,
    observed_thread_id: Option<String>,
    turn_id: Option<String>,
    observed_turn_id: Option<String>,
    lifecycle_starts_seen: [bool; 2],
    pending_terminal: Option<(String, CodexBrokerTerminal)>,
    terminal_emitted: bool,
}

impl CodexBrokerProtocol {
    pub(crate) fn new(
        codex_home: PathBuf,
        cwd: PathBuf,
        model: impl Into<String>,
    ) -> Result<Self, i32> {
        let model = model.into();
        if !codex_home.is_absolute()
            || !cwd.is_absolute()
            || model != "gpt-5.3-codex-spark"
            || model.len() > 256
        {
            return Err(66);
        }
        let codex_home = fs::canonicalize(codex_home).map_err(|_| 66)?;
        let cwd = fs::canonicalize(cwd).map_err(|_| 66)?;
        Ok(Self {
            codex_home,
            cwd,
            model,
            requests_sent: [false; 3],
            responses_seen: [false; 3],
            thread_id: None,
            observed_thread_id: None,
            turn_id: None,
            observed_turn_id: None,
            lifecycle_starts_seen: [false; 2],
            pending_terminal: None,
            terminal_emitted: false,
        })
    }

    pub(crate) fn mark_request_sent(&mut self, request: CodexBrokerRequest) -> Result<(), i32> {
        let index = request.index();
        if self.requests_sent[index]
            || (request == CodexBrokerRequest::ThreadStart && !self.responses_seen[0])
            || (request == CodexBrokerRequest::TurnStart && self.thread_id.is_none())
        {
            return Err(request.error_code());
        }
        self.requests_sent[index] = true;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn ingest_json_line(
        &mut self,
        bytes: &[u8],
    ) -> Result<Option<CodexBrokerTerminal>, i32> {
        if bytes.is_empty() || bytes.len() > MAX_CODEX_FRAME_BYTES || bytes.contains(&b'\n') {
            return Err(81);
        }
        let value = parse_bounded_codex_json(bytes).map_err(|_| 82)?;
        let kind = classify_codex_app_server_envelope(&value).map_err(|_| 75)?;
        self.ingest_frame(&ReceivedCodexFrame { kind, value })
    }

    #[allow(clippy::too_many_lines)]
    fn ingest_frame(
        &mut self,
        frame: &ReceivedCodexFrame,
    ) -> Result<Option<CodexBrokerTerminal>, i32> {
        if let CodexAppServerFrameKind::Lifecycle { ref method } = frame.kind {
            let object = frame
                .value
                .as_object()
                .ok_or_else(|| if method == "turn/completed" { 77 } else { 76 })?;
            classify_notification(object)
                .map_err(|_| if method == "turn/completed" { 77 } else { 76 })?;
        }
        match &frame.kind {
            CodexAppServerFrameKind::Response { id } => self.ingest_response(*id, &frame.value)?,
            CodexAppServerFrameKind::ServerRequest { .. } => return Err(74),
            CodexAppServerFrameKind::Lifecycle { method } if method == "thread/started" => {
                if !self.requests_sent[CodexBrokerRequest::ThreadStart.index()]
                    || self.lifecycle_starts_seen[0]
                {
                    return Err(76);
                }
                let params = frame
                    .value
                    .get("params")
                    .and_then(Value::as_object)
                    .ok_or(76)?;
                let thread = params.get("thread").and_then(Value::as_object).ok_or(76)?;
                let id = validate_thread_object(thread, &self.cwd).map_err(|_| 76)?;
                self.bind_thread_id(&id, 76)?;
                self.lifecycle_starts_seen[0] = true;
            }
            CodexAppServerFrameKind::Lifecycle { method } if method == "turn/started" => {
                self.require_turn_request()?;
                if self.lifecycle_starts_seen[1] {
                    return Err(76);
                }
                let params = frame
                    .value
                    .get("params")
                    .and_then(Value::as_object)
                    .ok_or(76)?;
                self.validate_thread_binding(params, 76)?;
                let turn = params.get("turn").and_then(Value::as_object).ok_or(76)?;
                validate_turn_object(turn).map_err(|_| 76)?;
                for item in turn.get("items").and_then(Value::as_array).ok_or(76)? {
                    validate_safe_item(item).map_err(|_| 76)?;
                }
                let id = required_nonempty_string(turn, "id").map_err(|_| 76)?;
                self.bind_turn_id(&id, 76)?;
                self.lifecycle_starts_seen[1] = true;
            }
            CodexAppServerFrameKind::Lifecycle { method } if method == "turn/completed" => {
                self.require_turn_request()?;
                if self.pending_terminal.is_some() {
                    return Err(88);
                }
                let params = frame
                    .value
                    .get("params")
                    .and_then(Value::as_object)
                    .ok_or(77)?;
                self.validate_thread_binding(params, 77)?;
                let turn = params.get("turn").and_then(Value::as_object).ok_or(77)?;
                let id = required_nonempty_string(turn, "id").map_err(|_| 77)?;
                if self
                    .observed_turn_id
                    .as_deref()
                    .is_some_and(|expected| expected != id)
                    || self
                        .turn_id
                        .as_deref()
                        .is_some_and(|expected| expected != id)
                {
                    return Err(77);
                }
                let terminal = validate_terminal_evidence(
                    &frame.value,
                    self.thread_id.as_deref().ok_or(77)?,
                    &id,
                )
                .map_err(|_| 77)?;
                self.pending_terminal = Some((id, terminal));
            }
            CodexAppServerFrameKind::Lifecycle { method } => {
                let params = frame
                    .value
                    .get("params")
                    .and_then(Value::as_object)
                    .ok_or(76)?;
                if method == "account/rateLimits/updated" {
                    self.require_turn_request()?;
                } else {
                    if method == "thread/status/changed" {
                        if !self.requests_sent[CodexBrokerRequest::ThreadStart.index()] {
                            return Err(76);
                        }
                    } else {
                        self.require_turn_request()?;
                    }
                    self.validate_thread_binding(params, 76)?;
                    if let Some(turn_id) = params.get("turnId").and_then(Value::as_str) {
                        self.bind_turn_id(turn_id, 76)?;
                    }
                }
            }
        }
        self.reconcile()
    }

    fn ingest_response(&mut self, id: i64, value: &Value) -> Result<(), i32> {
        let (request, index) = match id {
            0 => (CodexBrokerRequest::Initialize, 0),
            1 => (CodexBrokerRequest::ThreadStart, 1),
            2 => (CodexBrokerRequest::TurnStart, 2),
            _ => return Err(75),
        };
        if !self.requests_sent[index] || self.responses_seen[index] {
            return Err(request.error_code());
        }
        self.responses_seen[index] = true;
        match request {
            CodexBrokerRequest::Initialize => {
                validate_initialize_response(value, &self.codex_home).map_err(|_| 71)?;
            }
            CodexBrokerRequest::ThreadStart => {
                let id = validate_thread_start_response(value, &self.cwd, &self.model)
                    .map_err(|_| 72)?;
                if self
                    .observed_thread_id
                    .as_deref()
                    .is_some_and(|observed| observed != id)
                {
                    return Err(76);
                }
                self.thread_id = Some(id);
            }
            CodexBrokerRequest::TurnStart => {
                let id = validate_turn_start_response(value).map_err(|_| 73)?;
                if self
                    .observed_turn_id
                    .as_deref()
                    .is_some_and(|observed| observed != id)
                    || self
                        .pending_terminal
                        .as_ref()
                        .is_some_and(|(observed, _)| observed != &id)
                {
                    return Err(76);
                }
                self.turn_id = Some(id);
            }
        }
        Ok(())
    }

    fn validate_thread_binding(&self, params: &Map<String, Value>, code: i32) -> Result<(), i32> {
        let observed = params.get("threadId").and_then(Value::as_str).ok_or(code)?;
        let expected = self
            .thread_id
            .as_deref()
            .or(self.observed_thread_id.as_deref());
        if expected != Some(observed) {
            return Err(code);
        }
        Ok(())
    }

    fn bind_thread_id(&mut self, observed: &str, code: i32) -> Result<(), i32> {
        if self
            .thread_id
            .as_deref()
            .or(self.observed_thread_id.as_deref())
            .is_some_and(|expected| expected != observed)
        {
            return Err(code);
        }
        if self.observed_thread_id.is_none() {
            self.observed_thread_id = Some(observed.to_owned());
        }
        Ok(())
    }

    fn bind_turn_id(&mut self, observed: &str, code: i32) -> Result<(), i32> {
        if observed.is_empty()
            || observed.len() > 4_096
            || self
                .turn_id
                .as_deref()
                .or(self.observed_turn_id.as_deref())
                .is_some_and(|expected| expected != observed)
        {
            return Err(code);
        }
        if self.observed_turn_id.is_none() {
            self.observed_turn_id = Some(observed.to_owned());
        }
        Ok(())
    }

    fn require_turn_request(&self) -> Result<(), i32> {
        self.requests_sent[CodexBrokerRequest::TurnStart.index()]
            .then_some(())
            .ok_or(76)
    }

    fn active_turn_id(&self) -> Option<&str> {
        self.turn_id
            .as_deref()
            .or(self.observed_turn_id.as_deref())
            .or_else(|| {
                self.pending_terminal
                    .as_ref()
                    .map(|(turn_id, _)| turn_id.as_str())
            })
    }

    fn reconcile(&mut self) -> Result<Option<CodexBrokerTerminal>, i32> {
        if self.terminal_emitted {
            return Ok(None);
        }
        let Some(turn_id) = self.turn_id.as_deref() else {
            return Ok(None);
        };
        let Some((terminal_turn_id, terminal)) = self.pending_terminal.as_ref() else {
            return Ok(None);
        };
        if terminal_turn_id != turn_id {
            return Err(77);
        }
        self.terminal_emitted = true;
        Ok(Some(terminal.clone()))
    }
}

#[cfg(windows)]
fn validate_terminal_evidence(
    value: &Value,
    expected_thread: &str,
    expected_turn: &str,
) -> Result<CodexBrokerTerminal, i32> {
    let params = value.get("params").and_then(Value::as_object).ok_or(68)?;
    if params.get("threadId").and_then(Value::as_str) != Some(expected_thread) {
        return Err(68);
    }
    let turn = params.get("turn").and_then(Value::as_object).ok_or(68)?;
    validate_turn_object(turn)?;
    if turn.get("id").and_then(Value::as_str) != Some(expected_turn) {
        return Err(68);
    }
    let status = turn.get("status").and_then(Value::as_str).ok_or(68)?;
    let items = turn.get("items").and_then(Value::as_array).ok_or(68)?;
    let mut agent_message_count = 0_u64;
    let mut output = None;
    for item in items {
        validate_safe_item(item)?;
        if item.get("type").and_then(Value::as_str) == Some("agentMessage") {
            agent_message_count = agent_message_count.checked_add(1).ok_or(68)?;
            output = Some(
                item.get("text")
                    .and_then(Value::as_str)
                    .ok_or(68)?
                    .to_owned(),
            );
        }
    }
    Ok(CodexBrokerTerminal {
        agent_message_count,
        output: output.unwrap_or_else(|| "{}".to_owned()),
        status: status.to_owned(),
    })
}

#[cfg(windows)]
fn success_result(value: &Value, expected_id: i64) -> Result<&Map<String, Value>, i32> {
    let object = value.as_object().ok_or(68)?;
    if object.get("id").and_then(Value::as_i64) != Some(expected_id) || object.contains_key("error")
    {
        return Err(68);
    }
    object.get("result").and_then(Value::as_object).ok_or(68)
}

#[cfg(windows)]
fn validate_turn_object(turn: &Map<String, Value>) -> Result<(), i32> {
    crate::require_only_keys(
        turn,
        &[
            "completedAt",
            "durationMs",
            "error",
            "id",
            "items",
            "itemsView",
            "startedAt",
            "status",
        ],
        "HERMES_CODEX_BROKER_FATAL_FRAME",
    )
    .map_err(|_| 68)?;
    required_nonempty_string(turn, "id")?;
    if !turn.get("items").is_some_and(Value::is_array)
        || !matches!(
            turn.get("status").and_then(Value::as_str),
            Some("completed" | "interrupted" | "failed" | "inProgress")
        )
    {
        return Err(68);
    }
    Ok(())
}

#[cfg(windows)]
fn validate_read_only_sandbox(value: &Value) -> Result<(), i32> {
    let sandbox = value.as_object().ok_or(68)?;
    crate::require_only_keys(
        sandbox,
        &["type", "networkAccess"],
        "HERMES_CODEX_BROKER_FATAL_FRAME",
    )
    .map_err(|_| 68)?;
    if sandbox.get("type").and_then(Value::as_str) != Some("readOnly")
        || sandbox
            .get("networkAccess")
            .is_some_and(|value| value.as_bool() != Some(false))
    {
        return Err(68);
    }
    Ok(())
}

#[cfg(windows)]
fn validate_safe_item(value: &Value) -> Result<(), i32> {
    let item = value.as_object().ok_or(68)?;
    match item.get("type").and_then(Value::as_str) {
        Some("userMessage") => {
            require_only_item_keys(item, &["clientId", "content", "id", "type"])?;
            if !item.get("content").is_some_and(Value::is_array) {
                return Err(68);
            }
        }
        Some("agentMessage") => {
            require_only_item_keys(item, &["id", "memoryCitation", "phase", "text", "type"])?;
            if !item.get("text").is_some_and(Value::is_string) {
                return Err(68);
            }
        }
        Some("reasoning") => {
            require_only_item_keys(item, &["content", "id", "summary", "type"])?;
        }
        _ => return Err(68),
    }
    required_nonempty_string(item, "id")?;
    Ok(())
}

#[cfg(windows)]
fn require_only_item_keys(item: &Map<String, Value>, allowed: &[&str]) -> Result<(), i32> {
    if item
        .keys()
        .any(|key| !allowed.iter().any(|allowed| key == allowed))
    {
        return Err(68);
    }
    Ok(())
}

#[cfg(windows)]
fn require_exact_keys(object: &Map<String, Value>, expected: &[&str]) -> Result<(), i32> {
    if object.len() != expected.len()
        || object
            .keys()
            .any(|key| !expected.iter().any(|expected| key == expected))
    {
        return Err(68);
    }
    Ok(())
}

#[cfg(windows)]
fn required_nonempty_string(object: &Map<String, Value>, key: &str) -> Result<String, i32> {
    object
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty() && value.len() <= 4096)
        .map(ToOwned::to_owned)
        .ok_or(68)
}

#[cfg(windows)]
fn empty_tree_identity(root: &Path) -> Result<(String, u64), i32> {
    let mut entries = fs::read_dir(root).map_err(|_| 70)?;
    if entries.next().transpose().map_err(|_| 70)?.is_some() {
        return Err(70);
    }
    Ok((sha256_bytes(b"lattice.hermes.codex-empty-tree.v1\0"), 0))
}

#[cfg(windows)]
fn same_canonical_path(value: &str, expected: &Path) -> bool {
    fs::canonicalize(value)
        .ok()
        .is_some_and(|observed| crate::same_path(&observed, expected))
}

#[cfg(windows)]
fn ensure_before_helper_deadline(deadline: Instant) -> Result<(), i32> {
    (Instant::now() < deadline).then_some(()).ok_or(69)
}

#[cfg(windows)]
fn canonical_request_path(value: &str, file: bool) -> HermesAdapterResult<PathBuf> {
    let path = Path::new(value);
    if !path.is_absolute() {
        return Err(configuration("HERMES_CODEX_BROKER_PATH_REJECTED"));
    }
    let canonical =
        fs::canonicalize(path).map_err(|_| configuration("HERMES_CODEX_BROKER_PATH_REJECTED"))?;
    crate::reject_link_or_reparse_ancestors(&canonical)?;
    if (file && !canonical.is_file()) || (!file && !canonical.is_dir()) {
        return Err(configuration("HERMES_CODEX_BROKER_PATH_REJECTED"));
    }
    Ok(canonical)
}

#[cfg(windows)]
fn read_bounded_helper_file(path: &Path, limit: usize) -> Result<Vec<u8>, ()> {
    let metadata = fs::symlink_metadata(path).map_err(|_| ())?;
    if !metadata.file_type().is_file() || metadata.len() == 0 || metadata.len() > limit as u64 {
        return Err(());
    }
    let capacity = usize::try_from(metadata.len()).map_err(|_| ())?;
    let mut bytes = Vec::with_capacity(capacity);
    File::open(path)
        .map_err(|_| ())?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ())?;
    if bytes.len() as u64 != metadata.len() {
        return Err(());
    }
    Ok(bytes)
}

#[cfg(windows)]
fn config_lock_override(path: &Path) -> HermesAdapterResult<String> {
    let text = path_text(path)?.replace('\\', "/");
    let encoded = serde_json::to_string(&text)
        .map_err(|_| configuration("HERMES_CODEX_CONFIG_LOCK_PATH_REJECTED"))?;
    Ok(format!("debug.config_lockfile.load_path={encoded}"))
}

#[cfg(windows)]
fn codex_child_environment(
    launcher: &Path,
    codex_home: &Path,
    temp: &Path,
) -> HermesAdapterResult<BTreeMap<OsString, OsString>> {
    let mut environment = BTreeMap::new();
    for name in ["SystemRoot", "WINDIR", "ComSpec", "PATHEXT"] {
        if let Some(value) = std::env::var_os(name) {
            environment.insert(OsString::from(name), value);
        }
    }
    let launcher_parent = launcher
        .parent()
        .ok_or_else(|| configuration("HERMES_CODEX_LAUNCHER_PARENT_REJECTED"))?;
    let mut path_entries = vec![launcher_parent.to_path_buf()];
    if let Some(root) = std::env::var_os("SystemRoot") {
        path_entries.push(PathBuf::from(root).join("System32"));
    }
    environment.insert(
        OsString::from("PATH"),
        std::env::join_paths(path_entries)
            .map_err(|_| configuration("HERMES_CODEX_MINIMAL_PATH_REJECTED"))?,
    );
    environment.insert(
        OsString::from("CODEX_HOME"),
        codex_home.as_os_str().to_owned(),
    );
    for (name, value) in CodexBrokerPolicy::official().required_child_environment() {
        environment.insert(OsString::from(name), OsString::from(value));
    }
    environment.insert(OsString::from("NO_COLOR"), OsString::from("1"));
    environment.insert(OsString::from("TEMP"), temp.as_os_str().to_owned());
    environment.insert(OsString::from("TMP"), temp.as_os_str().to_owned());
    Ok(environment)
}

#[cfg(windows)]
fn validate_broker_codex_home(codex_home: &Path, product_root: &Path) -> HermesAdapterResult<()> {
    crate::reject_link_or_reparse_ancestors(codex_home)?;
    let home_metadata = fs::symlink_metadata(codex_home)
        .map_err(|_| configuration("HERMES_CODEX_HOME_REJECTED"))?;
    if !home_metadata.file_type().is_dir()
        || metadata_is_reparse(&home_metadata)
        || crate::path_is_within(codex_home, product_root)
        || crate::path_is_within(product_root, codex_home)
    {
        return Err(configuration("HERMES_CODEX_HOME_REJECTED"));
    }
    let ambient_home_matches = [
        std::env::var_os("CODEX_HOME").map(PathBuf::from),
        std::env::var_os("USERPROFILE").map(|root| PathBuf::from(root).join(".codex")),
        std::env::var_os("HOME").map(|root| PathBuf::from(root).join(".codex")),
    ]
    .into_iter()
    .flatten()
    .filter_map(|path| fs::canonicalize(path).ok())
    .any(|ambient| ambient == codex_home);
    if ambient_home_matches {
        return Err(configuration("HERMES_CODEX_HOME_AMBIENT_REJECTED"));
    }
    let marker = codex_home.join(CODEX_HOME_OWNERSHIP_MARKER_NAME);
    reject_reparse_to_boundary(&marker, codex_home)?;
    if bounded_file_bytes(&marker, 128)? != CODEX_HOME_OWNERSHIP_MARKER_BYTES {
        return Err(configuration("HERMES_CODEX_HOME_OWNERSHIP_REJECTED"));
    }
    let auth = codex_home.join("auth.json");
    reject_reparse_to_boundary(&auth, codex_home)?;
    let auth_metadata =
        fs::metadata(&auth).map_err(|_| configuration("HERMES_CODEX_HOME_AUTH_REJECTED"))?;
    if !auth_metadata.is_file()
        || auth_metadata.len() == 0
        || auth_metadata.len() > MAX_CODEX_AUTH_BYTES
    {
        return Err(configuration("HERMES_CODEX_HOME_AUTH_REJECTED"));
    }
    for forbidden in ["config.toml", "environments.toml"] {
        match fs::symlink_metadata(codex_home.join(forbidden)) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            _ => return Err(configuration("HERMES_CODEX_HOME_CONFIG_REJECTED")),
        }
    }
    Ok(())
}

#[cfg(windows)]
fn digest_environment(environment: &BTreeMap<OsString, OsString>) -> HermesAdapterResult<String> {
    let mut digest = Sha256::new();
    digest.update(b"lattice.hermes.codex-child-environment.v1\0");
    for (name, value) in environment {
        let name = name
            .to_str()
            .ok_or_else(|| configuration("HERMES_CODEX_ENVIRONMENT_REJECTED"))?;
        let value = value
            .to_str()
            .ok_or_else(|| configuration("HERMES_CODEX_ENVIRONMENT_REJECTED"))?;
        for field in [name.as_bytes(), value.as_bytes()] {
            digest.update((field.len() as u64).to_be_bytes());
            digest.update(field);
        }
    }
    Ok(encode_digest(&digest.finalize()))
}

#[cfg(windows)]
fn create_owned_directory(root: &Path, name: &str) -> HermesAdapterResult<PathBuf> {
    let path = root.join(name);
    fs::create_dir(&path).map_err(|_| spawn("HERMES_CODEX_BROKER_DIRECTORY_CREATE_FAILED"))?;
    Ok(path)
}

#[cfg(windows)]
fn write_new_file(path: &Path, bytes: &[u8]) -> HermesAdapterResult<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| spawn("HERMES_CODEX_BROKER_FILE_CREATE_FAILED"))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|_| spawn("HERMES_CODEX_BROKER_FILE_WRITE_FAILED"))
}

#[cfg(windows)]
fn path_text(path: &Path) -> HermesAdapterResult<String> {
    path.to_str()
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| configuration("HERMES_CODEX_BROKER_PATH_REJECTED"))
}

fn configuration(code: &'static str) -> HermesAdapterError {
    HermesAdapterError::new(HermesAdapterErrorKind::Configuration, code)
}

fn identity(code: &'static str) -> HermesAdapterError {
    HermesAdapterError::new(HermesAdapterErrorKind::Identity, code)
}

fn spawn(code: &'static str) -> HermesAdapterError {
    HermesAdapterError::new(HermesAdapterErrorKind::Spawn, code)
}

fn timeout(code: &'static str) -> HermesAdapterError {
    HermesAdapterError::new(HermesAdapterErrorKind::Timeout, code)
}

fn malformed_error(code: &'static str) -> HermesAdapterError {
    HermesAdapterError::new(HermesAdapterErrorKind::Malformed, code)
}

/// Fixed initialize/thread/turn requests for the no-marker canary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CodexNoMarkerCanaryPlan {
    cwd: PathBuf,
    nonce: String,
    model: String,
}

impl CodexNoMarkerCanaryPlan {
    pub(crate) fn new(
        cwd: PathBuf,
        nonce: impl Into<String>,
        model: impl Into<String>,
    ) -> HermesAdapterResult<Self> {
        let nonce = nonce.into();
        let model = model.into();
        if !cwd.is_absolute() || !is_lowercase_sha256(&nonce) || model != "gpt-5.3-codex-spark" {
            return Err(HermesAdapterError::new(
                HermesAdapterErrorKind::Configuration,
                "HERMES_CODEX_CANARY_PLAN_REJECTED",
            ));
        }
        Ok(Self { cwd, nonce, model })
    }

    #[allow(clippy::unused_self)]
    pub(crate) fn initialize_request(&self) -> Value {
        serde_json::json!({
            "id": 0,
            "method": "initialize",
            "params": {
                "clientInfo": {
                    "name": "lattice-hermes-reflection-broker",
                    "title": "LATTICE Hermes Reflection Broker",
                    "version": "1.0.0"
                },
                "capabilities": {
                    "experimentalApi": false,
                    "requestAttestation": false,
                    "mcpServerOpenaiFormElicitation": false
                }
            }
        })
    }

    #[allow(clippy::unused_self)]
    pub(crate) fn initialized_notification(&self) -> Value {
        serde_json::json!({"method": "initialized"})
    }

    pub(crate) fn thread_start_request(&self) -> Value {
        serde_json::json!({
            "id": 1,
            "method": "thread/start",
            "params": {
                "model": self.model,
                "cwd": self.cwd,
                "approvalPolicy": "never",
                "sandbox": "read-only",
                "serviceName": "lattice-hermes-reflection-canary",
                "ephemeral": true
            }
        })
    }

    pub(crate) fn turn_start_request(&self, thread_id: &str) -> Value {
        let marker = format!(".lattice-hermes-no-tools-{}", self.nonce);
        let prompt = format!(
            "Canary only. Do not create, modify, or inspect files. The marker {marker} must remain absent. Return exactly the schema-bound constant JSON. Do not add prose."
        );
        serde_json::json!({
            "id": 2,
            "method": "turn/start",
            "params": {
                "threadId": thread_id,
                "input": [{"type": "text", "text": prompt}],
                "model": self.model,
                "cwd": self.cwd,
                "approvalPolicy": "never",
                "sandboxPolicy": {"type": "readOnly", "networkAccess": false},
                "outputSchema": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["markerCreated", "nonce"],
                    "properties": {
                        "markerCreated": {"type": "boolean", "const": false},
                        "nonce": {"type": "string", "const": self.nonce}
                    }
                }
            }
        })
    }
}

/// Typed host observation for the no-marker canary. Raw transcript content is
/// never retained; only its digest crosses this constructor.
#[cfg(test)]
#[derive(Clone, Eq, PartialEq)]
pub(crate) struct CodexNoMarkerCanaryObservation {
    nonce: String,
    tree_sha256_before: String,
    tree_file_count_before: u64,
    tree_sha256_after: String,
    tree_file_count_after: u64,
    marker_existed_before: bool,
    marker_exists_after: bool,
    terminal_status: String,
    agent_message_count: u64,
    forbidden_event_count: u64,
    environment_connection_count: u64,
    output: Vec<u8>,
    transcript_sha256: String,
    descendants_reaped: bool,
}

#[cfg(test)]
impl std::fmt::Debug for CodexNoMarkerCanaryObservation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CodexNoMarkerCanaryObservation")
            .field("nonce", &self.nonce)
            .field("tree_file_count_before", &self.tree_file_count_before)
            .field("tree_file_count_after", &self.tree_file_count_after)
            .field("terminal_status", &self.terminal_status)
            .field("agent_message_count", &self.agent_message_count)
            .field("forbidden_event_count", &self.forbidden_event_count)
            .field(
                "environment_connection_count",
                &self.environment_connection_count,
            )
            .field("output_byte_count", &self.output.len())
            .field("transcript_sha256", &self.transcript_sha256)
            .field("descendants_reaped", &self.descendants_reaped)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
impl CodexNoMarkerCanaryObservation {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        nonce: impl Into<String>,
        tree_sha256_before: impl Into<String>,
        tree_file_count_before: u64,
        tree_sha256_after: impl Into<String>,
        tree_file_count_after: u64,
        marker_existed_before: bool,
        marker_exists_after: bool,
        terminal_status: impl Into<String>,
        agent_message_count: u64,
        forbidden_event_count: u64,
        environment_connection_count: u64,
        output: &[u8],
        transcript_sha256: impl Into<String>,
        descendants_reaped: bool,
    ) -> HermesAdapterResult<Self> {
        let observation = Self {
            nonce: nonce.into(),
            tree_sha256_before: tree_sha256_before.into(),
            tree_file_count_before,
            tree_sha256_after: tree_sha256_after.into(),
            tree_file_count_after,
            marker_existed_before,
            marker_exists_after,
            terminal_status: terminal_status.into(),
            agent_message_count,
            forbidden_event_count,
            environment_connection_count,
            output: output.to_vec(),
            transcript_sha256: transcript_sha256.into(),
            descendants_reaped,
        };
        if !is_lowercase_sha256(&observation.nonce)
            || !is_lowercase_sha256(&observation.tree_sha256_before)
            || !is_lowercase_sha256(&observation.tree_sha256_after)
            || !is_lowercase_sha256(&observation.transcript_sha256)
            || observation.output.is_empty()
            || observation.output.len() > 4096
        {
            return Err(HermesAdapterError::new(
                HermesAdapterErrorKind::Malformed,
                "HERMES_CODEX_CANARY_OBSERVATION_REJECTED",
            ));
        }
        Ok(observation)
    }
}

#[cfg(test)]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CodexNoMarkerCanaryReceipt {
    receipt_digest: ContentDigest,
    transcript_sha256: String,
}

#[cfg(test)]
impl CodexNoMarkerCanaryReceipt {
    pub(crate) const fn receipt_digest(&self) -> &ContentDigest {
        &self.receipt_digest
    }

    pub(crate) fn transcript_sha256(&self) -> &str {
        &self.transcript_sha256
    }
}

#[cfg(test)]
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct CanaryOutput {
    #[serde(rename = "markerCreated")]
    marker_created: bool,
    nonce: String,
}

#[cfg(test)]
pub(crate) fn verify_codex_no_marker_canary(
    observation: &CodexNoMarkerCanaryObservation,
) -> HermesAdapterResult<CodexNoMarkerCanaryReceipt> {
    let output: CanaryOutput = serde_json::from_slice(&observation.output).map_err(|_| {
        HermesAdapterError::new(
            HermesAdapterErrorKind::Malformed,
            "HERMES_CODEX_NO_MARKER_CANARY_REJECTED",
        )
    })?;
    let canonical = serde_json::to_vec(&output).map_err(|_| {
        HermesAdapterError::new(
            HermesAdapterErrorKind::Malformed,
            "HERMES_CODEX_NO_MARKER_CANARY_REJECTED",
        )
    })?;
    if canonical != observation.output
        || output.marker_created
        || output.nonce != observation.nonce
        || observation.marker_existed_before
        || observation.marker_exists_after
        || observation.tree_sha256_before != observation.tree_sha256_after
        || observation.tree_file_count_before != observation.tree_file_count_after
        || observation.terminal_status != "completed"
        || observation.agent_message_count != 1
        || observation.forbidden_event_count != 0
        || observation.environment_connection_count != 0
        || !observation.descendants_reaped
    {
        return Err(HermesAdapterError::new(
            HermesAdapterErrorKind::CapabilityMismatch,
            "HERMES_CODEX_NO_MARKER_CANARY_REJECTED",
        ));
    }
    let mut digest = Sha256::new();
    digest.update(b"lattice.hermes.codex-no-marker-receipt.v1\0");
    for field in [
        observation.nonce.as_bytes(),
        observation.tree_sha256_before.as_bytes(),
        observation.tree_file_count_before.to_string().as_bytes(),
        observation.transcript_sha256.as_bytes(),
        observation.output.as_slice(),
    ] {
        digest.update((field.len() as u64).to_be_bytes());
        digest.update(field);
    }
    let receipt_digest =
        ContentDigest::from_sha256(encode_digest(&digest.finalize())).map_err(|_| {
            HermesAdapterError::new(
                HermesAdapterErrorKind::Malformed,
                "HERMES_CODEX_NO_MARKER_CANARY_REJECTED",
            )
        })?;
    Ok(CodexNoMarkerCanaryReceipt {
        receipt_digest,
        transcript_sha256: observation.transcript_sha256.clone(),
    })
}

fn is_lowercase_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn encode_digest(digest: &[u8]) -> String {
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

/// Strict class of one accepted Codex app-server output frame.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum CodexAppServerFrameKind {
    Response { id: i64 },
    ServerRequest { id: i64, method: String },
    Lifecycle { method: String },
}

/// Parses a complete app-server JSON line. Any server request, tool-bearing
/// item, plan update, approval, auth refresh, or future discriminator is fatal
/// to the one-shot broker and therefore never forwarded to Hermes.
#[cfg(test)]
pub(crate) fn classify_codex_app_server_frame(
    bytes: &[u8],
) -> HermesAdapterResult<CodexAppServerFrameKind> {
    if bytes.is_empty() || bytes.len() > MAX_CODEX_FRAME_BYTES || bytes.contains(&b'\n') {
        return Err(fatal("HERMES_CODEX_BROKER_FATAL_FRAME"));
    }
    let value =
        parse_bounded_codex_json(bytes).map_err(|_| fatal("HERMES_CODEX_BROKER_FATAL_FRAME"))?;
    let kind = classify_codex_app_server_envelope(&value)?;
    if matches!(kind, CodexAppServerFrameKind::Lifecycle { .. }) {
        let object = value
            .as_object()
            .ok_or_else(|| fatal("HERMES_CODEX_BROKER_FATAL_FRAME"))?;
        return classify_notification(object);
    }
    Ok(kind)
}

fn classify_codex_app_server_envelope(
    value: &Value,
) -> HermesAdapterResult<CodexAppServerFrameKind> {
    let object = value
        .as_object()
        .ok_or_else(|| fatal("HERMES_CODEX_BROKER_FATAL_FRAME"))?;
    let has_id = object.contains_key("id");
    let has_method = object.contains_key("method");
    if has_id && has_method {
        return classify_server_request(object);
    }
    if has_id {
        return classify_response(object);
    }
    if has_method {
        return classify_notification_envelope(object);
    }
    Err(fatal("HERMES_CODEX_BROKER_FATAL_FRAME"))
}

fn classify_server_request(
    object: &Map<String, Value>,
) -> HermesAdapterResult<CodexAppServerFrameKind> {
    let keys = object.keys().map(String::as_str).collect::<HashSet<_>>();
    if keys != HashSet::from(["id", "method", "params"])
        || !object.get("params").is_some_and(Value::is_object)
    {
        return Err(fatal("HERMES_CODEX_BROKER_FATAL_FRAME"));
    }
    let id = object
        .get("id")
        .and_then(Value::as_i64)
        .filter(|id| *id >= 0)
        .ok_or_else(|| fatal("HERMES_CODEX_BROKER_FATAL_FRAME"))?;
    let method = object
        .get("method")
        .and_then(Value::as_str)
        .filter(|method| {
            !method.is_empty() && method.len() <= 256 && !method.chars().any(char::is_control)
        })
        .ok_or_else(|| fatal("HERMES_CODEX_BROKER_FATAL_FRAME"))?;
    Ok(CodexAppServerFrameKind::ServerRequest {
        id,
        method: method.to_owned(),
    })
}

fn classify_notification_envelope(
    object: &Map<String, Value>,
) -> HermesAdapterResult<CodexAppServerFrameKind> {
    let keys = object.keys().map(String::as_str).collect::<HashSet<_>>();
    if keys != HashSet::from(["method", "params"])
        || !object.get("params").is_some_and(Value::is_object)
    {
        return Err(fatal("HERMES_CODEX_BROKER_FATAL_FRAME"));
    }
    let method = object
        .get("method")
        .and_then(Value::as_str)
        .filter(|method| {
            matches!(
                *method,
                "thread/started"
                    | "turn/started"
                    | "account/rateLimits/updated"
                    | "thread/status/changed"
                    | "thread/tokenUsage/updated"
                    | "item/agentMessage/delta"
                    | "item/reasoning/textDelta"
                    | "item/reasoning/summaryPartAdded"
                    | "item/reasoning/summaryTextDelta"
                    | "item/started"
                    | "item/completed"
                    | "turn/completed"
            )
        })
        .ok_or_else(|| fatal("HERMES_CODEX_BROKER_FATAL_FRAME"))?;
    Ok(CodexAppServerFrameKind::Lifecycle {
        method: method.to_owned(),
    })
}

fn classify_response(object: &Map<String, Value>) -> HermesAdapterResult<CodexAppServerFrameKind> {
    let keys = object.keys().map(String::as_str).collect::<HashSet<_>>();
    let success = HashSet::from(["id", "result"]);
    let failure = HashSet::from(["id", "error"]);
    if keys != success && keys != failure {
        return Err(fatal("HERMES_CODEX_BROKER_FATAL_FRAME"));
    }
    let id = object
        .get("id")
        .and_then(Value::as_i64)
        .filter(|id| (0..=2).contains(id))
        .ok_or_else(|| fatal("HERMES_CODEX_BROKER_FATAL_FRAME"))?;
    Ok(CodexAppServerFrameKind::Response { id })
}

fn classify_notification(
    object: &Map<String, Value>,
) -> HermesAdapterResult<CodexAppServerFrameKind> {
    let keys = object.keys().map(String::as_str).collect::<HashSet<_>>();
    if keys != HashSet::from(["method", "params"]) {
        return Err(fatal("HERMES_CODEX_BROKER_FATAL_FRAME"));
    }
    let method = object
        .get("method")
        .and_then(Value::as_str)
        .ok_or_else(|| fatal("HERMES_CODEX_BROKER_FATAL_FRAME"))?;
    let params = object
        .get("params")
        .and_then(Value::as_object)
        .ok_or_else(|| fatal("HERMES_CODEX_BROKER_FATAL_FRAME"))?;
    match method {
        "thread/started" => require_control_keys(params, &["thread"])?,
        "account/rateLimits/updated" => {
            require_control_keys(params, &["rateLimits"])?;
            if !params.get("rateLimits").is_some_and(Value::is_object) {
                return Err(fatal("HERMES_CODEX_BROKER_FATAL_FRAME"));
            }
        }
        "turn/started" => require_control_keys(params, &["threadId", "turn"])?,
        "thread/status/changed" => require_control_keys(params, &["status", "threadId"])?,
        "thread/tokenUsage/updated" => {
            require_control_keys(params, &["threadId", "tokenUsage", "turnId"])?;
        }
        "item/agentMessage/delta" => {
            require_control_keys(params, &["delta", "itemId", "threadId", "turnId"])?;
        }
        "item/reasoning/textDelta" => {
            require_control_keys(
                params,
                &["contentIndex", "delta", "itemId", "threadId", "turnId"],
            )?;
        }
        "item/reasoning/summaryPartAdded" => {
            require_control_keys(params, &["itemId", "summaryIndex", "threadId", "turnId"])?;
        }
        "item/reasoning/summaryTextDelta" => {
            require_control_keys(
                params,
                &["delta", "itemId", "summaryIndex", "threadId", "turnId"],
            )?;
        }
        "item/started" | "item/completed" => {
            let timestamp = if method == "item/started" {
                "startedAtMs"
            } else {
                "completedAtMs"
            };
            require_control_keys(params, &[timestamp, "item", "threadId", "turnId"])?;
            validate_safe_control_item(
                params
                    .get("item")
                    .ok_or_else(|| fatal("HERMES_CODEX_BROKER_FATAL_FRAME"))?,
            )?;
        }
        "turn/completed" => validate_terminal_turn(params)?,
        _ => return Err(fatal("HERMES_CODEX_BROKER_FATAL_FRAME")),
    }
    Ok(CodexAppServerFrameKind::Lifecycle {
        method: method.to_owned(),
    })
}

fn validate_terminal_turn(params: &Map<String, Value>) -> HermesAdapterResult<()> {
    let params_keys = params.keys().map(String::as_str).collect::<HashSet<_>>();
    if params_keys != HashSet::from(["threadId", "turn"])
        || !params.get("threadId").is_some_and(Value::is_string)
    {
        return Err(fatal("HERMES_CODEX_BROKER_FATAL_FRAME"));
    }
    let turn = params
        .get("turn")
        .and_then(Value::as_object)
        .ok_or_else(|| fatal("HERMES_CODEX_BROKER_FATAL_FRAME"))?;
    let allowed_turn_fields = HashSet::from([
        "id",
        "items",
        "status",
        "completedAt",
        "durationMs",
        "error",
        "itemsView",
        "startedAt",
    ]);
    if turn
        .keys()
        .any(|key| !allowed_turn_fields.contains(key.as_str()))
        || !turn.get("id").is_some_and(Value::is_string)
        || !turn.get("status").is_some_and(Value::is_string)
    {
        return Err(fatal("HERMES_CODEX_BROKER_FATAL_FRAME"));
    }
    let items = turn
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| fatal("HERMES_CODEX_BROKER_FATAL_FRAME"))?;
    for item in items {
        validate_safe_control_item(item)?;
    }
    Ok(())
}

fn require_control_keys(object: &Map<String, Value>, expected: &[&str]) -> HermesAdapterResult<()> {
    if object.len() != expected.len()
        || object
            .keys()
            .any(|key| !expected.iter().any(|expected| key == expected))
    {
        return Err(fatal("HERMES_CODEX_BROKER_FATAL_FRAME"));
    }
    Ok(())
}

fn validate_safe_control_item(value: &Value) -> HermesAdapterResult<()> {
    let item = value
        .as_object()
        .ok_or_else(|| fatal("HERMES_CODEX_BROKER_FATAL_FRAME"))?;
    let (required, allowed): (&[&str], &[&str]) = match item.get("type").and_then(Value::as_str) {
        Some("userMessage") => (
            &["content", "id", "type"],
            &["clientId", "content", "id", "type"],
        ),
        Some("agentMessage") => (
            &["id", "text", "type"],
            &["id", "memoryCitation", "phase", "text", "type"],
        ),
        Some("reasoning") => (&["id", "type"], &["content", "id", "summary", "type"]),
        _ => return Err(fatal("HERMES_CODEX_BROKER_FATAL_FRAME")),
    };
    if required.iter().any(|key| !item.contains_key(*key))
        || item
            .keys()
            .any(|key| !allowed.iter().any(|allowed| key == allowed))
    {
        return Err(fatal("HERMES_CODEX_BROKER_FATAL_FRAME"));
    }
    Ok(())
}

fn fatal(code: &'static str) -> HermesAdapterError {
    HermesAdapterError::new(HermesAdapterErrorKind::CapabilityMismatch, code)
}

const MAX_CODEX_JSON_DEPTH: usize = 64;
const MAX_CODEX_JSON_NODES: usize = 65_536;
const MAX_CODEX_JSON_ARRAY_ITEMS: usize = 4_096;
const MAX_CODEX_JSON_OBJECT_FIELDS: usize = 1_024;
const MAX_CODEX_JSON_STRING_BYTES: usize = 256 * 1024;
const DUPLICATE_CODEX_KEY: &str = "LATTICE_DUPLICATE_CODEX_KEY";

struct CodexJsonStats {
    nodes: usize,
}

struct CodexValueSeed<'a> {
    depth: usize,
    stats: &'a mut CodexJsonStats,
}

impl<'de> DeserializeSeed<'de> for CodexValueSeed<'_> {
    type Value = Value;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        if self.depth > MAX_CODEX_JSON_DEPTH {
            return Err(de::Error::custom("LATTICE_CODEX_JSON_DEPTH"));
        }
        self.stats.nodes = self.stats.nodes.saturating_add(1);
        if self.stats.nodes > MAX_CODEX_JSON_NODES {
            return Err(de::Error::custom("LATTICE_CODEX_JSON_NODES"));
        }
        deserializer.deserialize_any(CodexValueVisitor {
            depth: self.depth,
            stats: self.stats,
        })
    }
}

struct CodexValueVisitor<'a> {
    depth: usize,
    stats: &'a mut CodexJsonStats,
}

impl<'de> Visitor<'de> for CodexValueVisitor<'_> {
    type Value = Value;

    fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("one bounded Codex JSON value")
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(Value::Null)
    }

    fn visit_none<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(Value::Null)
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(Value::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(Value::Number(value.into()))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(Value::Number(value.into()))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        serde_json::Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| E::custom("LATTICE_CODEX_JSON_NUMBER"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        if value.len() > MAX_CODEX_JSON_STRING_BYTES {
            return Err(E::custom("LATTICE_CODEX_JSON_STRING"));
        }
        Ok(Value::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        if value.len() > MAX_CODEX_JSON_STRING_BYTES {
            return Err(E::custom("LATTICE_CODEX_JSON_STRING"));
        }
        Ok(Value::String(value))
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element_seed(CodexValueSeed {
            depth: self.depth.saturating_add(1),
            stats: self.stats,
        })? {
            if values.len() == MAX_CODEX_JSON_ARRAY_ITEMS {
                return Err(de::Error::custom("LATTICE_CODEX_JSON_ARRAY"));
            }
            values.push(value);
        }
        Ok(Value::Array(values))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut object = Map::new();
        while let Some(key) = map.next_key::<String>()? {
            if key.len() > 256 || object.contains_key(&key) {
                return Err(de::Error::custom(DUPLICATE_CODEX_KEY));
            }
            if object.len() == MAX_CODEX_JSON_OBJECT_FIELDS {
                return Err(de::Error::custom("LATTICE_CODEX_JSON_OBJECT"));
            }
            let value = map.next_value_seed(CodexValueSeed {
                depth: self.depth.saturating_add(1),
                stats: self.stats,
            })?;
            object.insert(key, value);
        }
        Ok(Value::Object(object))
    }
}

fn parse_bounded_codex_json(bytes: &[u8]) -> Result<Value, serde_json::Error> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let mut stats = CodexJsonStats { nodes: 0 };
    let value = CodexValueSeed {
        depth: 0,
        stats: &mut stats,
    }
    .deserialize(&mut deserializer)?;
    deserializer.end()?;
    Ok(value)
}

#[cfg(all(test, windows))]
mod production_provider_tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::mpsc;

    struct ProviderFixture {
        config: CodexReflectionBrokerConfig,
        config_lock: PathBuf,
        receipt: CodexBrokerPreflightReceipt,
        root: PathBuf,
    }

    impl ProviderFixture {
        fn new() -> Self {
            let sequence = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("fixture clock")
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "lattice-hermes-codex-provider-{}-{sequence}",
                std::process::id()
            ));
            let bundle = root.join("bundle").join("x86_64-pc-windows-msvc");
            let launcher = bundle.join("bin").join("codex.exe");
            let helper = root.join("lattice-hermes-broker.exe");
            let codex_home = root.join("codex-home");
            let isolation_root = root.join("isolation");
            let product_root = root.join("product");
            let cwd = isolation_root.join("empty-work");
            let temp = isolation_root.join("temp");
            let config_lock = isolation_root.join("codex-reflection.lock.toml");
            for directory in [
                launcher.parent().expect("launcher parent"),
                &codex_home,
                &cwd,
                &temp,
                &product_root,
            ] {
                fs::create_dir_all(directory).expect("fixture directory");
            }
            fs::write(&launcher, b"fixture launcher").expect("fixture launcher");
            fs::write(&helper, b"fixture helper").expect("fixture helper");
            fs::write(
                codex_home.join(CODEX_HOME_OWNERSHIP_MARKER_NAME),
                CODEX_HOME_OWNERSHIP_MARKER_BYTES,
            )
            .expect("fixture ownership marker");
            fs::write(codex_home.join("auth.json"), b"{}").expect("fixture isolated auth state");
            fs::write(&config_lock, CODEX_CONFIG_LOCK.as_bytes()).expect("fixture config lock");

            let launcher = fs::canonicalize(launcher).expect("canonical fixture launcher");
            let helper = fs::canonicalize(helper).expect("canonical fixture helper");
            let codex_home = fs::canonicalize(codex_home).expect("canonical fixture home");
            let isolation_root =
                fs::canonicalize(isolation_root).expect("canonical fixture isolation");
            let product_root = fs::canonicalize(product_root).expect("canonical fixture product");
            let temp = fs::canonicalize(temp).expect("canonical fixture temp");
            let helper_sha256 =
                bounded_file_sha256(&helper, MAX_CODEX_LAUNCHER_BYTES).expect("helper identity");
            let child_environment = codex_child_environment(&launcher, &codex_home, &temp)
                .expect("fixture child environment");
            let child_environment_sha256 =
                digest_environment(&child_environment).expect("fixture environment digest");
            let config = CodexReflectionBrokerConfig::new(
                helper,
                helper_sha256.clone(),
                launcher,
                codex_home,
                isolation_root,
                product_root,
                "gpt-5.3-codex-spark",
            )
            .expect("fixture broker config");
            let receipt = CodexBrokerPreflightReceipt::test_only(
                child_environment_sha256,
                sha256_bytes(CODEX_CONFIG_LOCK.as_bytes()),
                helper_sha256,
                CODEX_LAUNCHER_SHA256.to_owned(),
            );
            Self {
                config,
                config_lock,
                receipt,
                root,
            }
        }
    }

    impl Drop for ProviderFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    struct ProcessFixture {
        process_probe: PathBuf,
        root: PathBuf,
        system_root: PathBuf,
    }

    impl ProcessFixture {
        fn new() -> Self {
            let sequence = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("process fixture clock")
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "lattice-hermes-codex-process-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&root).expect("process fixture root");
            let system_root = PathBuf::from(std::env::var_os("SystemRoot").expect("SystemRoot"));
            let process_probe = fs::canonicalize(
                system_root
                    .join("System32")
                    .join("WindowsPowerShell")
                    .join("v1.0")
                    .join("powershell.exe"),
            )
            .expect("canonical fixture PowerShell");
            Self {
                process_probe,
                root,
                system_root,
            }
        }

        fn provider(&self, stderr_limit: u64) -> (FixtureCodexProxyProvider, Arc<AtomicU32>) {
            let executable = fs::canonicalize(self.system_root.join("System32").join("more.com"))
                .expect("canonical fixture more.com");
            self.provider_for(&executable, Vec::new(), BTreeMap::new(), stderr_limit)
        }

        fn stderr_provider(
            &self,
            stderr_limit: u64,
        ) -> (FixtureCodexProxyProvider, Arc<AtomicU32>) {
            let executable = fs::canonicalize(self.system_root.join("System32").join("cmd.exe"))
                .expect("canonical fixture cmd.exe");
            let mut environment = BTreeMap::new();
            for name in ["SystemRoot", "WINDIR", "ComSpec", "PATHEXT"] {
                if let Some(value) = std::env::var_os(name) {
                    environment.insert(OsString::from(name), value);
                }
            }
            let command = "(for /L %i in (1,1,200) do @echo 0123456789abcdef0123456789abcdef 1>&2) & pause >NUL";
            self.provider_for(
                &executable,
                ["/D", "/Q", "/S", "/C", command]
                    .into_iter()
                    .map(OsString::from)
                    .collect(),
                environment,
                stderr_limit,
            )
        }

        fn provider_for(
            &self,
            executable: &Path,
            arguments: Vec<OsString>,
            environment: BTreeMap<OsString, OsString>,
            stderr_limit: u64,
        ) -> (FixtureCodexProxyProvider, Arc<AtomicU32>) {
            let process_id = Arc::new(AtomicU32::new(0));
            let plan = crate::windows_job::WindowsJobCommandPlan {
                executable: executable.to_path_buf(),
                arguments,
                current_dir: self.root.clone(),
                environment,
                run_root: self.root.clone(),
                stdout_path: self.root.join("fixture.stdout.unused"),
                stderr_path: self.root.join("fixture.stderr.unused"),
                stdout_limit: MAX_CODEX_FRAME_BYTES as u64,
                stderr_limit,
                deadline: Instant::now() + Duration::from_secs(10),
                teardown_timeout: Duration::from_secs(3),
            };
            (
                FixtureCodexProxyProvider {
                    control: Arc::new(OwnedCodexProxyControl::new(stderr_limit)),
                    executable_sha256: bounded_file_sha256(executable, MAX_CODEX_LAUNCHER_BYTES)
                        .expect("fixture executable identity"),
                    plan,
                    process_id: Arc::clone(&process_id),
                    stderr_limit,
                },
                process_id,
            )
        }
    }

    impl Drop for ProcessFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn process_has_exited(executable: &Path, process_id: u32) -> bool {
        let probe = format!(
            "if (Get-Process -Id {process_id} -ErrorAction SilentlyContinue) {{ exit 1 }} else {{ exit 0 }}"
        );
        Command::new(executable)
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                &probe,
            ])
            .status()
            .is_ok_and(|status| status.success())
    }

    #[test]
    fn production_provider_factory_rejects_model_receipt_and_lock_drift() {
        let fixture = ProviderFixture::new();
        fixture
            .config
            .clone()
            .into_production_proxy_provider_from_preflight(&fixture.receipt, "gpt-5.3-codex-spark")
            .expect("one receipt-bound official provider");

        let failure = fixture
            .config
            .clone()
            .into_production_proxy_provider_from_preflight(&fixture.receipt, "gpt-5.6-luna")
            .err()
            .expect("model substitution fails before process launch");
        assert_eq!(failure.kind(), HermesAdapterErrorKind::CrossBinding);
        assert_eq!(
            failure.code(),
            "HERMES_CODEX_PROXY_FACTORY_BINDING_REJECTED"
        );

        let mut wrong_receipt = fixture.receipt.clone();
        wrong_receipt.child_environment_sha256 = "d".repeat(64);
        let failure = fixture
            .config
            .clone()
            .into_production_proxy_provider_from_preflight(&wrong_receipt, "gpt-5.3-codex-spark")
            .err()
            .expect("receipt substitution fails before process launch");
        assert_eq!(failure.kind(), HermesAdapterErrorKind::CrossBinding);
        assert_eq!(
            failure.code(),
            "HERMES_CODEX_PROXY_FACTORY_BINDING_REJECTED"
        );

        fs::write(&fixture.config_lock, b"drifted lock").expect("drift fixture lock");
        let failure = fixture
            .config
            .clone()
            .into_production_proxy_provider_from_preflight(&fixture.receipt, "gpt-5.3-codex-spark")
            .err()
            .expect("config-lock drift fails before process launch");
        assert_eq!(failure.kind(), HermesAdapterErrorKind::Identity);
        assert_eq!(
            failure.code(),
            "HERMES_CODEX_PROXY_CONFIG_IDENTITY_REJECTED"
        );
    }

    #[test]
    fn production_provider_launch_plan_is_exact_and_config_lock_bound() {
        let fixture = ProviderFixture::new();
        let verified =
            VerifiedCodexProxyConfig::from_config(fixture.config.clone()).expect("verified config");
        let reviewed = ReviewedCodexBundle {
            launcher: verified.launcher.clone(),
            launcher_sha256: CODEX_LAUNCHER_SHA256.to_owned(),
            package_manifest_sha256: CODEX_PACKAGE_MANIFEST_SHA256.to_owned(),
        };
        let deadline = Instant::now() + Duration::from_secs(30);
        let plan = verified
            .command_plan(&reviewed, deadline)
            .expect("one exact official launch plan");
        let expected_override =
            config_lock_override(&verified.config_lock).expect("canonical sealed config override");
        assert_eq!(
            plan.arguments,
            [
                OsString::from("app-server"),
                OsString::from("--listen"),
                OsString::from("stdio://"),
                OsString::from("--strict-config"),
                OsString::from("-c"),
                OsString::from(expected_override),
                OsString::from("-c"),
                OsString::from(
                    "debug.config_lockfile.save_fields_resolved_from_model_catalog=false"
                ),
            ]
        );
        assert_eq!(plan.executable, verified.launcher);
        assert_eq!(plan.current_dir, verified.cwd);
        assert_eq!(plan.run_root, verified.isolation_root);
        assert_eq!(plan.deadline, deadline);
        assert_eq!(
            plan.environment
                .get(&OsString::from("CODEX_HOME"))
                .map(OsString::as_os_str),
            Some(verified.codex_home.as_os_str())
        );
        assert_eq!(
            plan.environment
                .get(&OsString::from("CODEX_EXEC_SERVER_URL"))
                .map(OsString::as_os_str),
            Some(std::ffi::OsStr::new("none"))
        );
        assert_eq!(
            plan.environment
                .get(&OsString::from(
                    "CODEX_INTERNAL_APP_SERVER_REMOTE_CONTROL_DISABLED"
                ))
                .map(OsString::as_os_str),
            Some(std::ffi::OsStr::new("1"))
        );
        for forbidden in [
            "OPENAI_API_KEY",
            "OPENAI_BASE_URL",
            "CODEX_API_KEY",
            "HERMES_API_KEY",
            "HERMES_MODEL",
        ] {
            assert!(!plan.environment.contains_key(&OsString::from(forbidden)));
        }
        assert_eq!(
            digest_environment(&plan.environment).expect("planned environment digest"),
            fixture.receipt.child_environment_sha256
        );
    }

    #[test]
    fn provider_control_retains_a_terminal_stderr_failure() {
        let mut state = OwnedCodexProxyState {
            child: None,
            stderr_evidence: None,
            stderr_limit: 1024,
            stderr_thread: Some(thread::spawn(|| Err(()))),
            terminal_failure: None,
        };
        let first = state
            .terminate()
            .expect_err("stderr drain failure is terminal");
        let repeated = state
            .terminate()
            .expect_err("terminal failure survives idempotent termination");
        assert_eq!(first, repeated);
        assert_eq!(first.kind(), HermesAdapterErrorKind::Transport);
        assert_eq!(first.code(), "HERMES_CODEX_PROXY_STDERR_DRAIN_FAILED");
    }

    #[test]
    fn process_fixture_relays_raw_bytes_and_shared_control_reaps_the_owned_job() {
        let fixture = ProcessFixture::new();
        let (provider, process_id) = fixture.provider(1024);
        let control = provider.control();
        let mut duplex = Box::new(provider)
            .open(Instant::now() + Duration::from_secs(10))
            .expect("open test-only Job-owned raw duplex");
        let mut reader = BufReader::new(duplex.take_reader().expect("take raw stdout"));
        let process_id = process_id.load(Ordering::Acquire);
        assert_ne!(process_id, 0);
        let payload = b"raw-jsonl-1\r\nraw-jsonl-2\r\n";
        duplex.write_all(payload).expect("write raw fixture bytes");
        let mut echoed = vec![0_u8; payload.len()];
        reader
            .read_exact(&mut echoed)
            .expect("read raw fixture bytes");
        assert_eq!(echoed, payload);

        drop(duplex);
        control
            .terminate()
            .expect("shared control terminates the exact Job");
        let mut trailing = Vec::new();
        reader
            .read_to_end(&mut trailing)
            .expect("Job termination closes raw stdout");
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline && !process_has_exited(&fixture.process_probe, process_id) {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(process_has_exited(&fixture.process_probe, process_id));
    }

    #[test]
    fn process_fixture_rejects_deadline_and_identity_before_spawn() {
        let fixture = ProcessFixture::new();
        let (provider, process_id) = fixture.provider(1024);
        let failure = Box::new(provider)
            .open(Instant::now())
            .err()
            .expect("expired deadline fails before spawn");
        assert_eq!(failure.kind(), HermesAdapterErrorKind::Timeout);
        assert_eq!(failure.code(), "HERMES_CODEX_PROXY_DEADLINE_EXCEEDED");
        assert_eq!(process_id.load(Ordering::Acquire), 0);

        let (mut provider, process_id) = fixture.provider(1024);
        provider.executable_sha256 = "0".repeat(64);
        let failure = Box::new(provider)
            .open(Instant::now() + Duration::from_secs(10))
            .err()
            .expect("fixture executable substitution fails before spawn");
        assert_eq!(failure.kind(), HermesAdapterErrorKind::Identity);
        assert_eq!(
            failure.code(),
            "HERMES_CODEX_PROXY_FIXTURE_IDENTITY_REJECTED"
        );
        assert_eq!(process_id.load(Ordering::Acquire), 0);
    }

    #[test]
    fn process_fixture_pre_open_cancel_prevents_spawn() {
        let fixture = ProcessFixture::new();
        let (provider, process_id) = fixture.provider(1024);
        let control = provider.control();
        control.terminate().expect("pre-open cancellation");
        let failure = Box::new(provider)
            .open(Instant::now() + Duration::from_secs(10))
            .err()
            .expect("cancelled provider cannot spawn");
        assert_eq!(failure.kind(), HermesAdapterErrorKind::Cancelled);
        assert_eq!(failure.code(), "HERMES_CODEX_PROXY_OPEN_CANCELLED");
        assert_eq!(process_id.load(Ordering::Acquire), 0);
    }

    #[test]
    fn process_fixture_post_spawn_check_is_cancellable_through_bound_control() {
        let fixture = ProcessFixture::new();
        let (provider, process_id) = fixture.provider(1024);
        let FixtureCodexProxyProvider {
            control,
            plan,
            stderr_limit,
            ..
        } = provider;
        let retained_control = Arc::clone(&control);
        let observed_process_id = Arc::clone(&process_id);
        let (entered_sender, entered_receiver) = mpsc::channel();
        let (release_sender, release_receiver) = mpsc::channel();
        let worker = thread::spawn(move || {
            launch_owned_proxy(
                &plan,
                stderr_limit,
                &control,
                move || {
                    entered_sender.send(()).expect("signal post-spawn check");
                    release_receiver.recv().expect("release post-spawn check");
                    Ok(())
                },
                move |observed| {
                    observed_process_id.store(observed, Ordering::Release);
                },
            )
        });
        entered_receiver
            .recv_timeout(Duration::from_secs(3))
            .expect("post-spawn check entered after control binding");
        let process_id = process_id.load(Ordering::Acquire);
        assert_ne!(process_id, 0);
        retained_control
            .terminate()
            .expect("shared control cancels blocked post-spawn check");
        assert!(process_has_exited(&fixture.process_probe, process_id));
        release_sender.send(()).expect("release post-spawn check");
        let failure = worker
            .join()
            .expect("post-spawn worker joins")
            .err()
            .expect("cancelled launch cannot return a duplex");
        assert_eq!(failure.kind(), HermesAdapterErrorKind::Cancelled);
        assert_eq!(failure.code(), "HERMES_CODEX_PROXY_OPEN_CANCELLED");
    }

    #[test]
    fn process_fixture_bounds_stderr_and_reaps_the_owned_job() {
        let fixture = ProcessFixture::new();
        let (provider, process_id) = fixture.stderr_provider(1024);
        let control = provider.control();
        let failure = match Box::new(provider).open(Instant::now() + Duration::from_secs(10)) {
            Err(failure) => failure,
            Ok(_duplex) => {
                let deadline = Instant::now() + Duration::from_secs(3);
                loop {
                    match control.ensure_running() {
                        Ok(()) if Instant::now() < deadline => {
                            thread::sleep(Duration::from_millis(10));
                        }
                        Ok(()) => panic!("stderr overflow was not detected"),
                        Err(failure) => break failure,
                    }
                }
            }
        };
        assert_eq!(failure.kind(), HermesAdapterErrorKind::Malformed);
        assert_eq!(failure.code(), "HERMES_CODEX_PROXY_STDERR_LIMIT_EXCEEDED");
        let _ = control.terminate();
        let process_id = process_id.load(Ordering::Acquire);
        assert_ne!(process_id, 0);
        let deadline = Instant::now() + Duration::from_secs(3);
        while Instant::now() < deadline && !process_has_exited(&fixture.process_probe, process_id) {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(process_has_exited(&fixture.process_probe, process_id));
    }
}
