//! Helm CLI write ops via a `helm` subprocess.
//!
//! Implementing Helm's templating + chart-deps + hook lifecycle natively in
//! Rust would be its own project, so for write paths (install/upgrade/
//! rollback/uninstall) Lumen shells out to the user's installed `helm` CLI.
//! Read paths (list/get/history) still decode the release Secret directly —
//! they don't need the helm binary.
//!
//! Each command streams stdout/stderr to a Tauri Channel so the UI can show
//! progress on long operations (chart download, hook waits). The underlying
//! `helm` process receives a private captured kubeconfig and explicit context
//! so it executes against the configuration authorized for that operation.
//!
//! `helm` must be on PATH; if it isn't we surface an actionable error
//! pointing at https://helm.sh/docs/intro/install/ rather than a generic
//! "command not found".

use crate::error::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use std::process::Stdio;
use tauri::ipc::Channel;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

const HELM_NOT_FOUND_HINT: &str =
    "helm CLI not found on PATH. Install from https://helm.sh/docs/intro/install/ or add it to your shell PATH.";

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HelmEvent {
    Stdout { line: String },
    Stderr { line: String },
    Exited { code: i32 },
    Error { message: String },
}

#[derive(Debug, Clone, Deserialize)]
pub struct HelmInstallRequest {
    pub release: String,
    pub chart: String,
    pub namespace: String,
    pub version: Option<String>,
    pub values_yaml: Option<String>,
    pub create_namespace: bool,
    pub wait: bool,
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HelmUpgradeRequest {
    pub release: String,
    pub chart: String,
    pub namespace: String,
    pub version: Option<String>,
    pub values_yaml: Option<String>,
    pub install: bool,
    pub wait: bool,
    pub atomic: bool,
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HelmRollbackRequest {
    pub release: String,
    pub namespace: String,
    pub revision: u32,
    pub wait: bool,
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HelmUninstallRequest {
    pub release: String,
    pub namespace: String,
    pub keep_history: bool,
}

/// Private immutable kubeconfig for one Helm operation. Its lifetime covers the
/// subprocess and its permissions prevent exposing embedded credentials.
pub struct ConfigSnapshot(std::path::PathBuf);
impl ConfigSnapshot {
    pub fn new(config: &kube::config::Kubeconfig) -> AppResult<Self> {
        use std::io::Write;
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let name = format!(
            "lumen-helm-config-{}-{}-{}.yaml",
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        );
        let path = std::env::temp_dir().join(name);
        let mut options = std::fs::OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .open(&path)
            .map_err(|e| AppError::Internal(e.to_string()))?;
        let snapshot = Self(path);
        file.write_all(
            serde_yaml::to_string(config)
                .map_err(|e| AppError::Internal(e.to_string()))?
                .as_bytes(),
        )
        .map_err(|e| AppError::Internal(e.to_string()))?;
        Ok(snapshot)
    }
}
impl Drop for ConfigSnapshot {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn helm_command(context: &str, snapshot: &ConfigSnapshot) -> AppResult<Command> {
    let mut cmd = Command::new("helm");
    cmd.arg("--kube-context")
        .arg(context)
        .arg("--kubeconfig")
        .arg(&snapshot.0);
    cmd.kill_on_drop(true);
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    Ok(cmd)
}

pub async fn install(
    context: &str,
    snapshot: ConfigSnapshot,
    req: HelmInstallRequest,
    channel: Channel<HelmEvent>,
    cancel: CancellationToken,
) -> AppResult<()> {
    let mut cmd = helm_command(context, &snapshot)?;
    cmd.arg("install")
        .arg(&req.release)
        .arg(&req.chart)
        .arg("--namespace")
        .arg(&req.namespace);
    if req.create_namespace {
        cmd.arg("--create-namespace");
    }
    if let Some(v) = &req.version {
        cmd.arg("--version").arg(v);
    }
    if req.wait {
        cmd.arg("--wait");
    }
    if req.dry_run {
        cmd.arg("--dry-run");
    }
    run_with_values(cmd, req.values_yaml.as_deref(), channel, cancel).await
}

pub async fn upgrade(
    context: &str,
    snapshot: ConfigSnapshot,
    req: HelmUpgradeRequest,
    channel: Channel<HelmEvent>,
    cancel: CancellationToken,
) -> AppResult<()> {
    let mut cmd = helm_command(context, &snapshot)?;
    cmd.arg("upgrade")
        .arg(&req.release)
        .arg(&req.chart)
        .arg("--namespace")
        .arg(&req.namespace);
    if req.install {
        cmd.arg("--install");
    }
    if let Some(v) = &req.version {
        cmd.arg("--version").arg(v);
    }
    if req.wait {
        cmd.arg("--wait");
    }
    if req.atomic {
        cmd.arg("--atomic");
    }
    if req.dry_run {
        cmd.arg("--dry-run");
    }
    run_with_values(cmd, req.values_yaml.as_deref(), channel, cancel).await
}

pub async fn rollback(
    context: &str,
    snapshot: ConfigSnapshot,
    req: HelmRollbackRequest,
    channel: Channel<HelmEvent>,
    cancel: CancellationToken,
) -> AppResult<()> {
    let mut cmd = helm_command(context, &snapshot)?;
    cmd.arg("rollback")
        .arg(&req.release)
        .arg(req.revision.to_string())
        .arg("--namespace")
        .arg(&req.namespace);
    if req.wait {
        cmd.arg("--wait");
    }
    if req.dry_run {
        cmd.arg("--dry-run");
    }
    run(cmd, channel, cancel).await
}

pub async fn uninstall(
    context: &str,
    snapshot: ConfigSnapshot,
    req: HelmUninstallRequest,
    channel: Channel<HelmEvent>,
    cancel: CancellationToken,
) -> AppResult<()> {
    let mut cmd = helm_command(context, &snapshot)?;
    cmd.arg("uninstall")
        .arg(&req.release)
        .arg("--namespace")
        .arg(&req.namespace);
    if req.keep_history {
        cmd.arg("--keep-history");
    }
    run(cmd, channel, cancel).await
}

async fn run_with_values(
    mut cmd: Command,
    values_yaml: Option<&str>,
    channel: Channel<HelmEvent>,
    cancel: CancellationToken,
) -> AppResult<()> {
    // Helm's `--values -` reads YAML from stdin, which avoids writing the
    // (potentially sensitive) values to a temp file on disk.
    let values = values_yaml.map(str::to_owned);
    if values.is_some() {
        cmd.arg("--values").arg("-");
        cmd.stdin(Stdio::piped());
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(AppError::K8s(HELM_NOT_FOUND_HINT.into()));
        }
        Err(e) => return Err(AppError::K8s(format!("spawn helm: {e}"))),
    };

    if let Some(v) = values {
        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            if let Err(e) = stdin.write_all(v.as_bytes()).await {
                let _ = channel.send(HelmEvent::Error {
                    message: format!("write values to helm stdin: {e}"),
                });
            }
        }
    }

    pump_to_channel(child, channel, cancel).await
}

async fn run(
    cmd: Command,
    channel: Channel<HelmEvent>,
    cancel: CancellationToken,
) -> AppResult<()> {
    let mut cmd = cmd;
    let child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(AppError::K8s(HELM_NOT_FOUND_HINT.into()));
        }
        Err(e) => return Err(AppError::K8s(format!("spawn helm: {e}"))),
    };
    pump_to_channel(child, channel, cancel).await
}

async fn pump_to_channel(
    mut child: tokio::process::Child,
    channel: Channel<HelmEvent>,
    cancel: CancellationToken,
) -> AppResult<()> {
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let ch = channel.clone();
    let ch2 = channel.clone();
    let c = cancel.clone();
    let c2 = cancel.clone();

    let stdout_task = stdout.map(|out| {
        tokio::spawn(async move {
            let mut reader = BufReader::new(out).lines();
            loop {
                tokio::select! {
                    _ = c.cancelled() => break,
                    line = reader.next_line() => match line {
                        Ok(Some(line)) => { let _ = ch.send(HelmEvent::Stdout { line }); }
                        Ok(None) | Err(_) => break,
                    }
                }
            }
        })
    });

    let stderr_task = stderr.map(|err| {
        tokio::spawn(async move {
            let mut reader = BufReader::new(err).lines();
            loop {
                tokio::select! {
                    _ = c2.cancelled() => break,
                    line = reader.next_line() => match line {
                        Ok(Some(line)) => { let _ = ch2.send(HelmEvent::Stderr { line }); }
                        Ok(None) | Err(_) => break,
                    }
                }
            }
        })
    });

    let exit_code = tokio::select! {
        _ = cancel.cancelled() => {
            let _ = child.kill().await;
            let _ = channel.send(HelmEvent::Error { message: "cancelled by user".into() });
            -1
        }
        status = child.wait() => match status {
            Ok(s) => s.code().unwrap_or(-1),
            Err(e) => {
                let _ = channel.send(HelmEvent::Error { message: format!("wait: {e}") });
                -1
            }
        }
    };

    if let Some(t) = stdout_task {
        let _ = t.await;
    }
    if let Some(t) = stderr_task {
        let _ = t.await;
    }

    let _ = channel.send(HelmEvent::Exited { code: exit_code });
    Ok(())
}

// ─── Read-only chart discovery helpers ────────────────────────────────────
//
// `helm search repo` and `helm show values` are synchronous — we capture the
// full output and parse it. They power the install/upgrade wizards' chart
// pickers and default-values seeding.
//
// Both are used purely for UX hints and never modify cluster state, so they
// run as plain `output()` calls without the streaming Channel apparatus
// that install/upgrade/rollback need.

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ChartHit {
    pub name: String,
    pub version: String,
    pub app_version: String,
    pub description: String,
}

#[derive(Debug, Deserialize)]
struct RawChartHit {
    name: String,
    version: String,
    #[serde(default)]
    app_version: String,
    #[serde(default)]
    description: String,
}

/// Parse `helm search repo --output json` into a flat ChartHit list.
///
/// helm emits an empty list (`null` or `[]`) when no repos are configured
/// or no chart matches the query — both are treated as "no results" rather
/// than an error so the wizard can still accept manual chart entry.
pub fn parse_search_output(json: &str) -> AppResult<Vec<ChartHit>> {
    let trimmed = json.trim();
    if trimmed.is_empty() || trimmed == "null" {
        return Ok(Vec::new());
    }
    let raw: Vec<RawChartHit> = serde_json::from_str(trimmed)
        .map_err(|e| AppError::Internal(format!("parse helm search output: {e}")))?;
    Ok(raw
        .into_iter()
        .map(|r| ChartHit {
            name: r.name,
            version: r.version,
            app_version: r.app_version,
            description: r.description,
        })
        .collect())
}

/// Run `helm search repo <query> --versions --output json`.
///
/// `query` is passed verbatim to helm; an empty string lists every chart in
/// every configured repo. We always pass `--versions` so a single chart with
/// multiple available versions shows up as multiple hits — the wizard groups
/// them client-side for its version picker.
pub async fn search_repo(query: &str) -> AppResult<Vec<ChartHit>> {
    let mut cmd = Command::new("helm");
    cmd.arg("search").arg("repo");
    if !query.is_empty() {
        cmd.arg(query);
    }
    cmd.arg("--versions").arg("--output").arg("json");
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let output = match cmd.output().await {
        Ok(o) => o,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(AppError::K8s(HELM_NOT_FOUND_HINT.into()));
        }
        Err(e) => return Err(AppError::K8s(format!("spawn helm: {e}"))),
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        // helm exits non-zero with "Error: no repositories configured" — treat
        // as empty so the wizard's manual-entry path stays usable instead of
        // erroring at the user.
        if stderr.contains("no repositories")
            || stderr.contains("no results found")
            || stderr.is_empty()
        {
            return Ok(Vec::new());
        }
        return Err(AppError::K8s(format!(
            "helm search repo exited {} — {}",
            output.status.code().unwrap_or(-1),
            stderr
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_search_output(&stdout)
}

/// Run `helm show values <chart> [--version <v>]` and return the YAML body.
///
/// helm's stdout for `show values` is the chart's default `values.yaml` —
/// what the install wizard pre-fills its values editor with so users see
/// every knob the chart exposes rather than having to read the registry.
pub async fn show_values(chart: &str, version: Option<&str>) -> AppResult<String> {
    if chart.trim().is_empty() {
        return Err(AppError::K8s("chart must not be empty".into()));
    }
    let mut cmd = Command::new("helm");
    cmd.arg("show").arg("values").arg(chart);
    if let Some(v) = version {
        if !v.is_empty() {
            cmd.arg("--version").arg(v);
        }
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let output = match cmd.output().await {
        Ok(o) => o,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(AppError::K8s(HELM_NOT_FOUND_HINT.into()));
        }
        Err(e) => return Err(AppError::K8s(format!("spawn helm: {e}"))),
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(AppError::K8s(format!(
            "helm show values exited {} — {}",
            output.status.code().unwrap_or(-1),
            stderr
        )));
    }

    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_search_output_empty_inputs() {
        assert!(parse_search_output("").unwrap().is_empty());
        assert!(parse_search_output("   \n").unwrap().is_empty());
        assert!(parse_search_output("null").unwrap().is_empty());
        assert!(parse_search_output("[]").unwrap().is_empty());
    }

    #[test]
    fn parse_search_output_typical_helm_json() {
        // Shape from `helm search repo bitnami/redis --versions --output json`.
        let json = r#"[
            {"name":"bitnami/redis","version":"19.0.1","app_version":"7.2.4","description":"Redis(R) is an open source, advanced key-value store."},
            {"name":"bitnami/redis","version":"18.19.4","app_version":"7.2.4","description":"Redis(R) is an open source, advanced key-value store."}
        ]"#;
        let hits = parse_search_output(json).unwrap();
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].name, "bitnami/redis");
        assert_eq!(hits[0].version, "19.0.1");
        assert_eq!(hits[0].app_version, "7.2.4");
        assert!(hits[0].description.contains("Redis"));
        assert_eq!(hits[1].version, "18.19.4");
    }

    #[test]
    fn parse_search_output_tolerates_missing_optional_fields() {
        // Some chart authors omit appVersion / description.
        let json = r#"[{"name":"acme/foo","version":"0.1.0"}]"#;
        let hits = parse_search_output(json).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "acme/foo");
        assert_eq!(hits[0].app_version, "");
        assert_eq!(hits[0].description, "");
    }

    #[test]
    fn parse_search_output_rejects_garbage() {
        assert!(parse_search_output("not json").is_err());
        // Object instead of array — shape mismatch.
        assert!(parse_search_output("{}").is_err());
    }
    #[test]
    fn helm_uses_private_snapshot_and_cleans_it_up() {
        let config = kube::config::Kubeconfig::default();
        let snapshot = super::ConfigSnapshot::new(&config).unwrap();
        let path = snapshot.0.clone();
        assert!(path.exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        let command = super::helm_command("prod", &snapshot).unwrap();
        let args: Vec<_> = command
            .as_std()
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            vec![
                "--kube-context",
                "prod",
                "--kubeconfig",
                path.to_str().unwrap()
            ]
        );
        drop(snapshot);
        assert!(!path.exists());
    }
}

// Repository settings use Helm's own local configuration and credential handling.
// Serialize operations to avoid competing writes to repositories.yaml/index caches.
static REPOSITORY_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
const REPOSITORY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HelmRepository {
    pub name: String,
    pub url: String,
}

fn validate_repository_name(name: &str) -> AppResult<()> {
    if name.is_empty()
        || name.len() > 128
        || !name.as_bytes()[0].is_ascii_alphanumeric()
        || !name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))
    {
        return Err(AppError::K8s("Repository name must start with a letter or number and contain only letters, numbers, dots, underscores or hyphens (maximum 128 characters).".into()));
    }
    Ok(())
}

fn validate_repository_url(url: &str) -> AppResult<()> {
    let uri = url
        .parse::<http::Uri>()
        .map_err(|_| AppError::K8s("Enter a valid HTTP(S) repository URL.".into()))?;
    if !matches!(uri.scheme_str(), Some("https" | "http"))
        || uri.host().is_none_or(str::is_empty)
        || uri.authority().is_some_and(|a| a.as_str().contains('@'))
        || uri.query().is_some()
        || url.contains('#')
        || url.contains('\\')
        || url.chars().any(char::is_whitespace)
    {
        return Err(AppError::K8s("Use an HTTP(S) repository URL without embedded credentials, query parameters or fragments. Configure authenticated repositories with your local Helm CLI.".into()));
    }
    Ok(())
}

fn parse_repository_output(json: &str) -> AppResult<Vec<HelmRepository>> {
    if json.trim().is_empty() || json.trim() == "null" {
        return Ok(vec![]);
    }
    let mut repositories: Vec<HelmRepository> = serde_json::from_str(json)
        .map_err(|_| AppError::K8s("Unable to read Helm repository list.".into()))?;
    // Existing CLI configuration may contain credentials. Never send them to the UI.
    for repo in &mut repositories {
        repo.url = repo
            .url
            .parse::<http::Uri>()
            .ok()
            .and_then(|uri| {
                let scheme = uri.scheme_str()?;
                let authority = uri.authority()?.as_str().rsplit('@').next()?;
                Some(format!("{scheme}://{authority}{}", uri.path()))
            })
            .unwrap_or_else(|| "URL hidden (unsupported format)".into());
    }
    Ok(repositories)
}

async fn repository_output(args: &[&str]) -> AppResult<std::process::Output> {
    let mut command = Command::new("helm");
    command.args(args).kill_on_drop(true).stdin(Stdio::null());
    tokio::time::timeout(REPOSITORY_TIMEOUT, command.output()).await
        .map_err(|_| AppError::K8s("Helm repository operation timed out after 60 seconds. Check connectivity and retry.".into()))?
        .map_err(|e| if e.kind() == std::io::ErrorKind::NotFound {
            AppError::K8s(HELM_NOT_FOUND_HINT.into())
        } else { AppError::K8s("Unable to run the local Helm CLI.".into()) })
}

fn repository_result(output: std::process::Output) -> AppResult<()> {
    if output.status.success() {
        Ok(())
    } else {
        // Helm stderr may include credentials from an existing repository URL.
        Err(AppError::K8s("Helm repository operation failed. Check the repository name, URL, connectivity and authentication using your local Helm CLI.".into()))
    }
}

pub async fn list_repositories() -> AppResult<Vec<HelmRepository>> {
    let _lock = REPOSITORY_LOCK.lock().await;
    let output = repository_output(&["repo", "list", "--output", "json"]).await?;
    if !output.status.success() {
        if String::from_utf8_lossy(&output.stderr).contains("no repositories") {
            return Ok(vec![]);
        }
        repository_result(output)?;
        return Ok(vec![]);
    }
    parse_repository_output(&String::from_utf8_lossy(&output.stdout))
}

pub async fn add_repository(name: &str, url: &str) -> AppResult<()> {
    validate_repository_name(name)?;
    validate_repository_url(url)?;
    let _lock = REPOSITORY_LOCK.lock().await;
    repository_result(repository_output(&["repo", "add", name, url]).await?)
}

pub async fn remove_repository(name: &str) -> AppResult<()> {
    validate_repository_name(name)?;
    let _lock = REPOSITORY_LOCK.lock().await;
    repository_result(repository_output(&["repo", "remove", name]).await?)
}

pub async fn update_repositories() -> AppResult<()> {
    let _lock = REPOSITORY_LOCK.lock().await;
    // Fail when any repository update fails instead of reporting partial success.
    repository_result(repository_output(&["repo", "update", "--fail-on-repo-update-fail"]).await?)
}

#[cfg(test)]
mod repository_tests {
    use super::*;

    #[test]
    fn repository_names_cannot_be_flags_or_paths() {
        for name in ["", "--debug", "a/b", "a b", ".", "a\n"] {
            assert!(validate_repository_name(name).is_err(), "{name}");
        }
        for name in ["bitnami", "my-repo_2.local"] {
            assert!(validate_repository_name(name).is_ok());
        }
    }

    #[test]
    fn repository_urls_require_http_without_credentials() {
        for url in [
            "--debug",
            "file:///tmp/chart",
            "oci://host/chart",
            "https://u:p@host/chart",
            "https://host/chart?token=secret",
            "https://host/#secret",
            "https://",
            "https://host/ bad",
        ] {
            assert!(validate_repository_url(url).is_err(), "{url}");
        }
        assert!(validate_repository_url("https://charts.example.org/stable").is_ok());
        assert!(validate_repository_url("http://localhost:8080/charts").is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn repository_failure_does_not_expose_cli_credentials() {
        use std::os::unix::process::ExitStatusExt;
        let output = std::process::Output {
            status: std::process::ExitStatus::from_raw(256),
            stdout: b"secret-output".to_vec(),
            stderr: b"failed https://user:secret-password@host/repo".to_vec(),
        };
        let error = repository_result(output).unwrap_err().to_string();
        assert!(error.contains("operation failed"));
        assert!(!error.contains("secret"));
        assert!(!error.contains("user:"));
    }

    #[test]
    fn repository_list_redacts_existing_credentials_and_queries() {
        let repos = parse_repository_output(r#"[{"name":"private","url":"https://user:secret@charts.example.org/repo?token=secret"}]"#).unwrap();
        assert_eq!(repos[0].url, "https://charts.example.org/repo");
        assert!(parse_repository_output("[]").unwrap().is_empty());
        assert!(parse_repository_output("null").unwrap().is_empty());
        assert!(parse_repository_output("invalid").is_err());
    }
}
