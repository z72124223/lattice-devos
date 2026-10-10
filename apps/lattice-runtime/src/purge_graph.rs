//! Read-only ownership proof for current and retained Graphify source inputs.
use super::{
    digest, graph_executable_sha256, graph_git_output,
    graphify_platform_selection_from_environment, graphify_runtime_root_from_environment,
    path_text, project_purge_graph_configuration_digests,
};
use lattice_cjson::CanonicalValue;
use lattice_postgres_store::GraphSourceOwnership;
use std::{
    env, fs,
    path::{Path, PathBuf},
};

type Result<T> = std::result::Result<T, &'static str>;

struct HistoricalSource {
    source_root: PathBuf,
    runtime_root: PathBuf,
    git_executable: PathBuf,
}

fn canonical(path: &Path) -> Result<PathBuf> {
    let root = fs::canonicalize(path).map_err(|_| "GRAPH_SOURCE_UNAVAILABLE")?;
    if !root.is_dir() {
        return Err("GRAPH_SOURCE_UNAVAILABLE");
    }
    Ok(root)
}

fn git_path(git: &Path, root: &Path, argument: &str) -> Result<PathBuf> {
    let output = graph_git_output(git, root, ["rev-parse", "--path-format=absolute", argument])
        .map_err(|_| "GRAPH_REPOSITORY_IDENTITY_UNAVAILABLE")?;
    let text = std::str::from_utf8(&output.stdout)
        .map_err(|_| "GRAPH_REPOSITORY_IDENTITY_UNAVAILABLE")?
        .trim_end_matches(['\r', '\n']);
    if text.is_empty() || text.contains(['\r', '\n', '\0']) {
        return Err("GRAPH_REPOSITORY_IDENTITY_UNAVAILABLE");
    }
    canonical(Path::new(text))
}

fn common(git: &Path, root: &Path) -> Result<Option<PathBuf>> {
    // Non-Git project roots keep their existing direct-source proof. Never accept
    // an enclosing repository as the identity of a nested directory.
    if !root.join(".git").exists() {
        return Ok(None);
    }
    if git_path(git, root, "--show-toplevel")? != root {
        return Err("GRAPH_SOURCE_NOT_REPOSITORY_ROOT");
    }
    git_path(git, root, "--git-common-dir").map(Some)
}

/// Recompute owned selectors from actual files; request JSON cannot assert hashes.
///
/// # Errors
/// Missing inputs, aliased repositories, unknown history or missing commits fail closed.
pub fn project_purge_graph_source(
    root: &str,
    analyses: &[(String, String)],
) -> Result<GraphSourceOwnership> {
    // Following an existing junction is read-only. Its resolved identity is compared
    // for every Registry survivor before the purger can select any data.
    let root = canonical(Path::new(root))?;
    let git = PathBuf::from(
        env::var_os("LATTICE_DELIVERY_GIT_EXE").ok_or("GRAPH_GIT_IDENTITY_UNAVAILABLE")?,
    );
    let repository = common(&git, &root)?;
    let mut values = project_purge_graph_configuration_digests(
        root.to_str().ok_or("GRAPH_SOURCE_UNAVAILABLE")?,
    )?;
    let sources = historical_sources(&root, &git)?;
    for source in sources {
        if !source.source_root.is_absolute()
            || !source.runtime_root.is_absolute()
            || !source.git_executable.is_absolute()
        {
            return Err("GRAPH_HISTORY_INPUT_REJECTED");
        }
        let source_root = canonical(&source.source_root)?;
        let source_common = common(&source.git_executable, &source_root)?;
        if repository.is_none() || source_common != repository {
            continue;
        }
        canonical(&source.runtime_root)?;
        let sha = graph_executable_sha256(&source.git_executable)
            .map_err(|_| "GRAPH_GIT_IDENTITY_UNAVAILABLE")?;
        let legacy = digest(
            "lattice.runtime.graphify-source-configuration",
            &CanonicalValue::Object(vec![
                ("git_sha256".into(), CanonicalValue::String(sha)),
                (
                    "repository_root".into(),
                    CanonicalValue::String(
                        path_text(&source_root).map_err(|_| "GRAPH_HISTORY_INPUT_REJECTED")?,
                    ),
                ),
                (
                    "runtime_root".into(),
                    CanonicalValue::String(
                        path_text(&source.runtime_root)
                            .map_err(|_| "GRAPH_HISTORY_INPUT_REJECTED")?,
                    ),
                ),
            ]),
        )
        .map_err(|_| "GRAPH_HISTORY_INPUT_REJECTED")?;
        let mut candidates = vec![legacy.as_str().to_owned()];
        if let Some(platform) = graphify_platform_selection_from_environment()
            .map_err(|_| "GRAPH_HISTORY_INPUT_REJECTED")?
        {
            candidates.push(
                digest(
                    "lattice.runtime.graphify-source-configuration.v2",
                    &CanonicalValue::Object(vec![
                        (
                            "legacy_configuration".into(),
                            CanonicalValue::String(legacy.as_str().to_owned()),
                        ),
                        (
                            "platform_selection".into(),
                            CanonicalValue::String(platform),
                        ),
                    ]),
                )
                .map_err(|_| "GRAPH_HISTORY_INPUT_REJECTED")?
                .as_str()
                .to_owned(),
            );
        }
        for (configuration, commit) in analyses
            .iter()
            .filter(|(config, _)| candidates.contains(config))
        {
            if commit.len() != 40 || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err("GRAPH_SOURCE_COMMIT_UNAVAILABLE");
            }
            graph_git_output(
                &source.git_executable,
                &source_root,
                ["cat-file", "-e", &format!("{commit}^{{commit}}")],
            )
            .map_err(|_| "GRAPH_SOURCE_COMMIT_UNAVAILABLE")?;
            values.push(configuration.clone());
        }
    }
    values.sort();
    values.dedup();
    Ok(GraphSourceOwnership {
        configurations: values,
        repository_identity: repository.map(|path| path.to_string_lossy().into_owned()),
    })
}

fn historical_sources(root: &Path, git: &Path) -> Result<Vec<HistoricalSource>> {
    let mut sources = Vec::new();
    if let Some(source) = env::var_os("LATTICE_GRAPHIFY_SOURCE_ROOT") {
        sources.push(HistoricalSource {
            source_root: PathBuf::from(source),
            runtime_root: graphify_runtime_root_from_environment(root),
            git_executable: git.to_path_buf(),
        });
    }
    if let Some(raw) = env::var_os("LATTICE_GRAPHIFY_PURGE_SOURCE_HISTORY") {
        let raw = raw.to_str().ok_or("GRAPH_HISTORY_INPUT_REJECTED")?;
        if raw.len() > 16384 {
            return Err("GRAPH_HISTORY_INPUT_REJECTED");
        }
        let history: Vec<serde_json::Value> =
            serde_json::from_str(raw).map_err(|_| "GRAPH_HISTORY_INPUT_REJECTED")?;
        if history.len() > 16 {
            return Err("GRAPH_HISTORY_INPUT_REJECTED");
        }
        for entry in history {
            let object = entry.as_object().ok_or("GRAPH_HISTORY_INPUT_REJECTED")?;
            if object.len() != 3
                || object
                    .keys()
                    .any(|k| !["sourceRoot", "runtimeRoot", "gitExecutable"].contains(&k.as_str()))
            {
                return Err("GRAPH_HISTORY_INPUT_REJECTED");
            }
            let path = |key: &str| {
                entry[key]
                    .as_str()
                    .filter(|value| !value.is_empty())
                    .map(PathBuf::from)
                    .ok_or("GRAPH_HISTORY_INPUT_REJECTED")
            };
            sources.push(HistoricalSource {
                source_root: path("sourceRoot")?,
                runtime_root: path("runtimeRoot")?,
                git_executable: path("gitExecutable")?,
            });
        }
    }
    Ok(sources)
}
