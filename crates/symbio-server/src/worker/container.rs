use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tempfile::TempDir;
use tokio::process::Command;
use url::Url;

use super::{BuildId, WorkerError, WorkerId};

pub const BROKER_PORT: u16 = 18791;
pub(crate) const DEFAULT_BROKER_STREAMS: usize = 32;
pub(crate) const MAX_BROKER_STREAMS: usize = 256;
const DOCKER_TIMEOUT: Duration = Duration::from_secs(15);
const DEFAULT_CPU: &str = "2";
const DEFAULT_MEMORY: &str = "2g";
const DEFAULT_PIDS: u32 = 128;
const DEFAULT_TMPFS: &str = "64m";
const SKIPPED_ROOTS: &[&str] = &["session"];
const CREDENTIAL_ROOTS: &[&str] = &[
    ".ssh", ".aws", ".azure", ".config", ".docker", ".gnupg", ".kube", "appdata",
];
const CREDENTIAL_FILES: &[&str] = &[
    ".env",
    ".npmrc",
    ".netrc",
    ".git-credentials",
    ".htpasswd",
    "credentials.json",
];
const DISABLED_PLUGINS: &[&str] = &["gateway", "telegram", "web"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkerMode {
    Host,
    Container,
}

#[derive(Debug, Clone)]
pub struct WorkerConfig {
    pub mode: WorkerMode,
    pub image: Option<String>,
    pub runtime: String,
    pub cpu: String,
    pub memory: String,
    pub pids: u32,
    pub tmpfs: String,
    pub max_broker_streams: usize,
}

impl Default for WorkerConfig {
    fn default() -> Self {
        Self {
            mode: WorkerMode::Host,
            image: None,
            runtime: "docker".to_string(),
            cpu: DEFAULT_CPU.to_string(),
            memory: DEFAULT_MEMORY.to_string(),
            pids: DEFAULT_PIDS,
            tmpfs: DEFAULT_TMPFS.to_string(),
            max_broker_streams: DEFAULT_BROKER_STREAMS,
        }
    }
}

impl WorkerConfig {
    pub fn from_env() -> Result<Self, WorkerError> {
        let mut config = Self::default();
        config.mode = match std::env::var("SYMBIO_WORKER_MODE").as_deref() {
            Ok("host") => WorkerMode::Host,
            Ok("container") => WorkerMode::Container,
            Ok(other) => return Err(invalid(format!("SYMBIO_WORKER_MODE 无效: {other}"))),
            Err(_) => WorkerMode::Host,
        };
        config.runtime = env_or("SYMBIO_DOCKER", &config.runtime);
        config.cpu = env_or("SYMBIO_WORKER_CPU", &config.cpu);
        config.memory = env_or("SYMBIO_WORKER_MEMORY", &config.memory);
        config.tmpfs = env_or("SYMBIO_WORKER_TMPFS", &config.tmpfs);
        config.image = std::env::var("SYMBIO_WORKER_IMAGE")
            .ok()
            .filter(|v| !v.trim().is_empty());
        if let Ok(value) = std::env::var("SYMBIO_WORKER_PIDS") {
            config.pids = value
                .parse()
                .map_err(|_| invalid("SYMBIO_WORKER_PIDS 必须是正整数"))?;
        }
        if let Ok(value) = std::env::var("SYMBIO_BROKER_MAX_STREAMS") {
            config.max_broker_streams = value
                .parse()
                .map_err(|_| invalid("SYMBIO_BROKER_MAX_STREAMS 必须是正整数"))?;
        }
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), WorkerError> {
        if self.runtime.trim().is_empty() || self.runtime != "docker" {
            return Err(invalid("仅支持 docker 运行时"));
        }
        if self.pids == 0
            || self.max_broker_streams == 0
            || self.max_broker_streams > MAX_BROKER_STREAMS
        {
            return Err(invalid(
                "PID、broker 流上限必须为正且 broker 上限不得超过 256",
            ));
        }
        validate_cpu(&self.cpu)?;
        validate_size(&self.memory)?;
        validate_size(&self.tmpfs)?;
        if self.mode == WorkerMode::Container {
            let image = self
                .image
                .as_deref()
                .ok_or_else(|| invalid("容器模式必须设置 SYMBIO_WORKER_IMAGE"))?;
            validate_image(image)?;
        }
        Ok(())
    }
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| default.to_string())
}

fn invalid(message: impl Into<String>) -> WorkerError {
    WorkerError::Invalid(message.into())
}

pub fn validate_image(image: &str) -> Result<(), WorkerError> {
    let Some((repository, digest)) = image.rsplit_once("@sha256:") else {
        return Err(invalid("容器镜像必须固定为 repository@sha256:<64 hex>"));
    };
    if repository.is_empty() || digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(invalid("容器镜像摘要格式无效"));
    }
    Ok(())
}

fn validate_cpu(value: &str) -> Result<(), WorkerError> {
    let valid = value.parse::<f64>().is_ok_and(|v| v > 0.0 && v <= 256.0);
    if !valid {
        return Err(invalid("CPU 上限必须是 (0, 256] 的数字"));
    }
    Ok(())
}

fn validate_size(value: &str) -> Result<(), WorkerError> {
    let digits: String = value.chars().take_while(|c| c.is_ascii_digit()).collect();
    let suffix = &value[digits.len()..];
    let valid = !digits.is_empty()
        && digits.parse::<u64>().is_ok_and(|v| v > 0)
        && matches!(suffix, "" | "k" | "m" | "g");
    if !valid {
        return Err(invalid(format!("资源大小无效: {value}")));
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct BrokerTarget {
    pub(super) id: String,
    pub(super) kind: BrokerKind,
    pub(super) scope: Vec<String>,
    pub(super) base: Url,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) timeout_secs: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BrokerKind {
    Model,
    Mcp,
}

impl BrokerTarget {
    fn prefix(&self) -> String {
        broker_path(self.kind, &self.scope, &self.id)
    }

    pub fn accepts_path(&self, path: &str) -> bool {
        let prefix = self.prefix();
        let path = path.split('?').next().unwrap_or(path);
        let Some(rest) = path.strip_prefix(&prefix) else {
            return false;
        };
        match self.kind {
            BrokerKind::Model => rest.is_empty() || rest.starts_with('/'),
            BrokerKind::Mcp => rest.is_empty(),
        }
    }

    pub fn url_for(&self, path: &str) -> Result<Url, String> {
        let prefix = self.prefix();
        let rest = path.strip_prefix(&prefix).ok_or("broker 目标路径不匹配")?;
        let (next_path, query) = rest.split_once('?').unwrap_or((rest, ""));
        if next_path.split('/').any(|segment| {
            let mut decoded = segment.to_string();
            for _ in 0..4 {
                let Ok(next) = urlencoding::decode(&decoded) else {
                    return true;
                };
                if next == decoded {
                    break;
                }
                decoded = next.into_owned();
            }
            matches!(decoded.as_str(), "." | "..") || decoded.contains(['/', '\\'])
        }) {
            return Err("broker 拒绝路径穿越".to_string());
        }
        if self.kind == BrokerKind::Mcp && !next_path.is_empty() {
            return Err("MCP broker 只接受配置端点".to_string());
        }
        let mut url = self.base.clone();
        let mut appended = url.path().trim_end_matches('/').to_string();
        appended.push_str(next_path);
        url.set_path(&appended);
        if !query.is_empty() {
            url.set_query(Some(query));
        }
        Ok(url)
    }
}

fn broker_path(kind: BrokerKind, scope: &[String], id: &str) -> String {
    let mut path = String::new();
    if !scope.is_empty() {
        path.push_str("/subagent");
        for agent in scope {
            path.push('/');
            path.push_str(&urlencoding::encode(agent));
        }
    }
    path.push_str(match kind {
        BrokerKind::Model => "/model/",
        BrokerKind::Mcp => "/mcp/",
    });
    path.push_str(&urlencoding::encode(id));
    path
}

fn broker_key(kind: BrokerKind, scope: &[String], id: &str) -> String {
    let mut key = String::new();
    for agent in scope {
        key.push_str("subagent:");
        key.push_str(agent);
        key.push('/');
    }
    key.push_str(match kind {
        BrokerKind::Model => "model:",
        BrokerKind::Mcp => "mcp:",
    });
    key.push_str(id);
    key
}

pub struct ContainerLaunch {
    command: Command,
    pub runtime: TempDir,
    pub targets: HashMap<String, BrokerTarget>,
    pub name: String,
    pub token: String,
}

impl ContainerLaunch {
    pub async fn spawn(&mut self) -> Result<tokio::process::Child, String> {
        self.command.spawn().map_err(|e| e.to_string())
    }
}

pub async fn prepare_container(
    config: &WorkerConfig,
    build: &BuildId,
    worker: &WorkerId,
    homedir: &Path,
    session_dir: &Path,
) -> Result<ContainerLaunch, WorkerError> {
    config.validate()?;
    let image = config
        .image
        .clone()
        .ok_or_else(|| invalid("缺少容器镜像"))?;
    verify_docker(config, &image, build).await?;
    let homedir = fs::canonicalize(homedir).map_err(|e| invalid(format!("homedir 不存在: {e}")))?;
    let session_dir =
        fs::canonicalize(session_dir).map_err(|e| invalid(format!("会话目录不存在: {e}")))?;
    let projection = TempDir::new().map_err(|e| invalid(format!("创建运行时投影失败: {e}")))?;
    let targets = project_runtime(&homedir, projection.path())?;
    let workdir = selected_workdir(&session_dir, &homedir)?;
    let user = container_user(&session_dir, workdir.as_deref())?;
    let token = uuid::Uuid::new_v4().simple().to_string();
    let name = format!(
        "symbio-worker-{}-{}",
        std::process::id(),
        uuid::Uuid::new_v4().simple()
    );
    let mut command = Command::new(&config.runtime);
    command.env_clear();
    copy_host_process_env(&mut command);
    command.args([
        "run",
        "--rm",
        "-i",
        "--name",
        &name,
        "--network",
        "none",
        "--read-only",
        "--cap-drop",
        "ALL",
        "--security-opt",
        "no-new-privileges",
        "--pids-limit",
        &config.pids.to_string(),
        "--cpus",
        &config.cpu,
        "--memory",
        &config.memory,
        "--memory-swap",
        &config.memory,
        "--tmpfs",
        &format!("/tmp:rw,noexec,nosuid,size={}", config.tmpfs),
        "--user",
        &user,
        "--mount",
        &mount(projection.path(), &homedir, true),
        "--mount",
        &mount(&session_dir, &session_dir, false),
    ]);
    if let Some(workdir) = workdir {
        command.args(["--mount", &mount(&workdir, &workdir, false)]);
    }
    command.args([
        "--env",
        "SYMBIO_PROCESS_ROLE=worker",
        "--env",
        &format!("SYMBIO_HOMEDIR={}", homedir.display()),
        "--env",
        &format!("SYMBIO_BROKER_TOKEN={token}"),
        "--env",
        &format!("SYMBIO_BROKER_STREAMS={}", config.max_broker_streams),
        &image,
        "__worker",
        "--session",
        worker.session_id(),
        "--session-dir",
        &session_dir.to_string_lossy(),
        "--epoch",
        &worker.epoch().value().to_string(),
        "--build",
        worker.build_id().as_str(),
        "--homedir",
        &homedir.to_string_lossy(),
    ]);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true);
    Ok(ContainerLaunch {
        command,
        runtime: projection,
        targets,
        name,
        token,
    })
}

fn copy_host_process_env(command: &mut Command) {
    copy_env_values(|key, value| {
        command.env(key, value);
    });
}

fn copy_env_values(mut set: impl FnMut(&str, std::ffi::OsString)) {
    for key in ["PATH", "SystemRoot", "WINDIR", "TEMP", "TMP"] {
        if let Some(value) = std::env::var_os(key) {
            set(key, value);
        }
    }
}

fn mount(source: &Path, target: &Path, readonly: bool) -> String {
    let source = source.to_string_lossy().replace(',', ",,");
    let target = target.to_string_lossy().replace(',', ",,");
    format!(
        "type=bind,source={source},target={target}{}",
        if readonly { ",readonly" } else { "" }
    )
}

async fn verify_docker(
    config: &WorkerConfig,
    image: &str,
    build: &BuildId,
) -> Result<(), WorkerError> {
    let output = docker_output(config, &["version", "--format", "{{.Server.Os}}"]).await?;
    let server_os = String::from_utf8_lossy(&output).trim().to_string();
    let host_os = host_os();
    let output = docker_output(config, &["info", "--format", "{{json .SecurityOptions}}"]).await?;
    let security_options: Vec<String> = serde_json::from_slice(&output)
        .map_err(|e| invalid(format!("读取 Docker 安全信息失败: {e}")))?;
    if security_options
        .iter()
        .any(|option| option.eq_ignore_ascii_case("name=rootless"))
    {
        return Err(invalid(
            "rootless Docker 的 UID 映射与宿主挂载权限不同，容器 worker 暂不支持",
        ));
    }
    let output = docker_output(config, &["image", "inspect", image]).await?;
    let inspect: Value =
        serde_json::from_slice(&output).map_err(|e| invalid(format!("读取镜像信息失败: {e}")))?;
    verify_image_metadata(&server_os, host_os, &inspect, image, build)
}

fn host_os() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "unsupported"
    }
}

fn verify_image_metadata(
    server_os: &str,
    host_os: &str,
    inspect: &Value,
    image: &str,
    build: &BuildId,
) -> Result<(), WorkerError> {
    if server_os != host_os {
        return Err(invalid(format!(
            "容器 OS {server_os} 与宿主 {host_os} 不一致"
        )));
    }
    let entry = inspect
        .as_array()
        .and_then(|v| v.first())
        .ok_or_else(|| invalid("镜像不存在"))?;
    let pinned = entry
        .pointer("/RepoDigests")
        .and_then(Value::as_array)
        .is_some_and(|v| v.iter().filter_map(Value::as_str).any(|v| v == image));
    let label = entry
        .pointer("/Config/Labels/symbio.build-id")
        .and_then(Value::as_str)
        == Some(build.as_str());
    if !pinned || !label {
        return Err(invalid("镜像摘要或 symbio.build-id 与宿主构建不匹配"));
    }
    Ok(())
}

async fn docker_output(config: &WorkerConfig, args: &[&str]) -> Result<Vec<u8>, WorkerError> {
    let mut command = Command::new(&config.runtime);
    command.env_clear();
    copy_host_process_env(&mut command);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let output = tokio::time::timeout(DOCKER_TIMEOUT, command.output())
        .await
        .map_err(|_| invalid("Docker 检查超时"))?
        .map_err(|e| invalid(format!("Docker 不可用: {e}")))?;
    if !output.status.success() {
        return Err(invalid(format!(
            "Docker 检查失败: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(output.stdout)
}

pub async fn remove_container(config: &WorkerConfig, name: &str) -> Result<(), WorkerError> {
    let mut command = Command::new(&config.runtime);
    command.env_clear();
    copy_host_process_env(&mut command);
    command
        .args(["rm", "-f", name])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    let output = tokio::time::timeout(DOCKER_TIMEOUT, command.output())
        .await
        .map_err(|_| invalid("Docker 容器清理超时"))?
        .map_err(|e| invalid(format!("Docker 容器清理失败: {e}")))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(invalid("Docker 容器清理失败"))
    }
}

fn is_credential_root(name: &str) -> bool {
    CREDENTIAL_ROOTS
        .iter()
        .any(|root| name.eq_ignore_ascii_case(root))
}

fn is_credential_file(name: &str) -> bool {
    CREDENTIAL_FILES
        .iter()
        .any(|file| name.eq_ignore_ascii_case(file))
        || [".pem", ".key", ".p12", ".pfx"]
            .iter()
            .any(|suffix| name.to_ascii_lowercase().ends_with(suffix))
}

fn is_credential_path_name(name: &str) -> bool {
    is_credential_root(name) || is_credential_file(name)
}

fn regular_file(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok_and(|meta| meta.file_type().is_file())
}

fn container_user(session_dir: &Path, workdir: Option<&Path>) -> Result<String, WorkerError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        let session =
            fs::metadata(session_dir).map_err(|e| invalid(format!("读取会话权限失败: {e}")))?;
        let uid = session.uid();
        if uid == 0 || uid != std::os::unix::process::euid() {
            return Err(invalid("容器 worker 要求会话由当前非 root 用户所有"));
        }
        if session.mode() & 0o200 == 0 {
            return Err(invalid("容器 worker 要求会话目录所有者可写"));
        }
        if let Some(workdir) = workdir {
            let metadata = fs::metadata(workdir)
                .map_err(|e| invalid(format!("读取 workdir 权限失败: {e}")))?;
            if metadata.uid() != uid || metadata.mode() & 0o300 != 0o300 {
                return Err(invalid(
                    "容器 workdir 必须由同一非 root 用户所有且可读写执行",
                ));
            }
        }
        return Ok(format!("{uid}:{}", session.gid()));
    }
    #[cfg(not(unix))]
    {
        let _ = (session_dir, workdir);
        Ok("65532:65532".to_string())
    }
}

fn reject_credential_tree(path: &Path) -> Result<(), WorkerError> {
    let mut pending = vec![path.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)
            .map_err(|e| invalid(format!("检查 workdir 凭证目录失败: {e}")))?
        {
            let entry = entry.map_err(|e| invalid(e.to_string()))?;
            let kind = entry.file_type().map_err(|e| invalid(e.to_string()))?;
            let name = entry.file_name();
            if is_credential_path_name(&name.to_string_lossy()) {
                return Err(invalid("workdir 内不得包含宿主凭证文件或目录"));
            }
            if !kind.is_dir() {
                continue;
            }
            pending.push(entry.path());
        }
    }
    Ok(())
}

fn selected_workdir(session_dir: &Path, homedir: &Path) -> Result<Option<PathBuf>, WorkerError> {
    let content = fs::read_to_string(session_dir.join("session.json"))
        .map_err(|e| invalid(format!("读取会话失败: {e}")))?;
    let meta: Value =
        serde_json::from_str(&content).map_err(|e| invalid(format!("解析会话失败: {e}")))?;
    let Some(value) = meta
        .pointer("/metadata/workdir")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
    else {
        return Ok(None);
    };
    let path = PathBuf::from(value);
    if !path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
    {
        return Err(invalid("workdir 必须是无 .. 的绝对路径"));
    }
    let canonical = fs::canonicalize(&path).map_err(|e| invalid(format!("workdir 不存在: {e}")))?;
    if !canonical.is_dir() {
        return Err(invalid("workdir 不是目录"));
    }
    if canonical == homedir
        || canonical.starts_with(session_dir)
        || homedir.starts_with(&canonical)
        || session_dir.starts_with(&canonical)
        || ["model", "mcp"]
            .iter()
            .any(|name| canonical.starts_with(homedir.join(name)))
        || canonical
            .components()
            .any(|component| is_credential_path_name(&component.as_os_str().to_string_lossy()))
    {
        return Err(invalid(
            "workdir 不得覆盖 homedir、会话树、模型/MCP 或凭证目录",
        ));
    }
    reject_credential_tree(&canonical)?;
    Ok(Some(path))
}

fn project_runtime(
    homedir: &Path,
    target: &Path,
) -> Result<HashMap<String, BrokerTarget>, WorkerError> {
    let mut targets = HashMap::new();
    project_scope(homedir, target, &[], true, &mut targets)?;
    fs::create_dir_all(target.join("session")).map_err(|e| invalid(e.to_string()))?;
    let session_manifest = homedir.join("session/PLUGIN.yml");
    if regular_file(&session_manifest) {
        project_plugin_manifest(&session_manifest, &target.join("session/PLUGIN.yml"))?;
    }
    Ok(targets)
}

fn project_scope(
    source: &Path,
    target: &Path,
    scope: &[String],
    system_root: bool,
    targets: &mut HashMap<String, BrokerTarget>,
) -> Result<(), WorkerError> {
    fs::create_dir_all(target).map_err(|e| invalid(e.to_string()))?;
    let manifest = source.join("PLUGIN.yml");
    if regular_file(&manifest) {
        project_plugin_manifest(&manifest, &target.join("PLUGIN.yml"))?;
    }
    for entry in fs::read_dir(source).map_err(|e| invalid(format!("读取投影目录失败: {e}")))?
    {
        let entry = entry.map_err(|e| invalid(e.to_string()))?;
        if !entry
            .file_type()
            .map_err(|e| invalid(e.to_string()))?
            .is_dir()
        {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if SKIPPED_ROOTS.contains(&name.as_str()) || is_credential_root(&name) {
            continue;
        }
        let path = entry.path();
        let destination = target.join(&name);
        match name.as_str() {
            "model" => project_models(&path, target, scope, targets)?,
            "mcp" => project_mcp(&path, target, scope, targets)?,
            "subagent" => project_agents(&path, &destination, scope, targets)?,
            "skill" | "skills" => copy_tree(&path, &destination)?,
            _ if system_root && !regular_file(&path.join("PLUGIN.yml")) => continue,
            _ => copy_tree(&path, &destination)?,
        }
    }
    Ok(())
}

fn project_agents(
    source: &Path,
    target: &Path,
    scope: &[String],
    targets: &mut HashMap<String, BrokerTarget>,
) -> Result<(), WorkerError> {
    fs::create_dir_all(target).map_err(|e| invalid(e.to_string()))?;
    for entry in fs::read_dir(source).map_err(|e| invalid(e.to_string()))? {
        let entry = entry.map_err(|e| invalid(e.to_string()))?;
        if !entry
            .file_type()
            .map_err(|e| invalid(e.to_string()))?
            .is_dir()
        {
            continue;
        }
        let id = entry.file_name().to_string_lossy().to_string();
        validate_resource_id(&id)?;
        let mut nested = scope.to_vec();
        nested.push(id.clone());
        project_scope(&entry.path(), &target.join(id), &nested, false, targets)?;
    }
    Ok(())
}

fn copy_tree(source: &Path, target: &Path) -> Result<(), WorkerError> {
    fs::create_dir_all(target).map_err(|e| invalid(e.to_string()))?;
    for entry in fs::read_dir(source).map_err(|e| invalid(e.to_string()))? {
        let entry = entry.map_err(|e| invalid(e.to_string()))?;
        let kind = entry.file_type().map_err(|e| invalid(e.to_string()))?;
        let path = entry.path();
        let destination = target.join(entry.file_name());
        if kind.is_dir() {
            if !is_credential_root(&entry.file_name().to_string_lossy()) {
                copy_tree(&path, &destination)?;
            }
        } else if kind.is_file() {
            if entry.file_name() == "PLUGIN.yml" {
                project_plugin_manifest(&path, &destination)?;
            } else {
                fs::copy(&path, &destination).map_err(|e| invalid(e.to_string()))?;
            }
        }
    }
    Ok(())
}

fn project_plugin_manifest(source: &Path, target: &Path) -> Result<(), WorkerError> {
    let content = fs::read_to_string(source).map_err(|e| invalid(e.to_string()))?;
    let manifest: serde_yaml_ng::Value =
        serde_yaml_ng::from_str(&content).map_err(|e| invalid(format!("PLUGIN.yml 无效: {e}")))?;
    let mut value = serde_json::to_value(manifest).map_err(|e| invalid(e.to_string()))?;
    redact_secrets(&mut value);
    if let Some(object) = value.as_object_mut() {
        object.remove("env");
        if object
            .get("plugin_provider")
            .and_then(Value::as_str)
            .is_some_and(|id| DISABLED_PLUGINS.contains(&id))
        {
            object.insert("enabled".to_string(), Value::Bool(false));
        }
    }
    let output = serde_yaml_ng::to_string(
        &serde_yaml_ng::to_value(value).map_err(|e| invalid(e.to_string()))?,
    )
    .map_err(|e| invalid(e.to_string()))?;
    fs::write(target, output).map_err(|e| invalid(e.to_string()))
}

fn redact_secrets(value: &mut Value) {
    match value {
        Value::Object(object) => {
            object.retain(|key, _| !is_secret_key(key));
            for value in object.values_mut() {
                redact_secrets(value);
            }
        }
        Value::Array(values) => values.iter_mut().for_each(redact_secrets),
        _ => {}
    }
}

fn is_secret_key(key: &str) -> bool {
    let normalized: String = key
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    normalized == "env"
        || normalized == "headers"
        || normalized.ends_with("apikey")
        || normalized.ends_with("privatekey")
        || normalized.ends_with("token")
        || normalized.ends_with("secret")
        || normalized.ends_with("password")
        || normalized.ends_with("credential")
        || normalized.ends_with("credentials")
        || matches!(normalized.as_str(), "authorization" | "cookie" | "oauth")
}

fn validate_resource_id(id: &str) -> Result<(), WorkerError> {
    if id.is_empty()
        || matches!(id, "." | "..")
        || id.contains(['/', '\\'])
        || id.chars().any(char::is_control)
    {
        return Err(invalid("模型或 MCP id 无效"));
    }
    Ok(())
}

fn validate_trusted_headers(headers: &[(String, String)]) -> Result<(), WorkerError> {
    for (name, value) in headers {
        reqwest::header::HeaderName::from_bytes(name.as_bytes())
            .map_err(|_| invalid("凭证头名称无效"))?;
        reqwest::header::HeaderValue::from_str(value).map_err(|_| invalid("凭证头值无效"))?;
    }
    Ok(())
}

fn project_models(
    source: &Path,
    target: &Path,
    scope: &[String],
    targets: &mut HashMap<String, BrokerTarget>,
) -> Result<(), WorkerError> {
    if !source.is_dir() {
        return Ok(());
    }
    let destination = target.join("model");
    fs::create_dir_all(&destination).map_err(|e| invalid(e.to_string()))?;
    let manifest = source.join("PLUGIN.yml");
    if regular_file(&manifest) {
        project_plugin_manifest(&manifest, &destination.join("PLUGIN.yml"))?;
    }
    for entry in fs::read_dir(source).map_err(|e| invalid(e.to_string()))? {
        let entry = entry.map_err(|e| invalid(e.to_string()))?;
        if !entry
            .file_type()
            .map_err(|e| invalid(e.to_string()))?
            .is_dir()
        {
            continue;
        }
        let id = entry.file_name().to_string_lossy().to_string();
        let file = entry.path().join("provider.json");
        if !regular_file(&file) {
            continue;
        }
        let mut config: Value =
            serde_json::from_slice(&fs::read(&file).map_err(|e| invalid(e.to_string()))?)
                .map_err(|e| invalid(format!("provider.json 无效: {e}")))?;
        let Some(base) = config.get("api_base").and_then(Value::as_str) else {
            return Err(invalid(format!("模型 {id} 缺少 api_base")));
        };
        let base = Url::parse(base).map_err(|e| invalid(format!("模型 {id} URL 无效: {e}")))?;
        if !matches!(base.scheme(), "http" | "https")
            || base.host_str().is_none()
            || !base.username().is_empty()
            || base.password().is_some()
        {
            return Err(invalid(format!("模型 {id} URL 无效")));
        }
        validate_resource_id(&id)?;
        let protocol = config
            .get("api_protocol")
            .and_then(Value::as_str)
            .unwrap_or("openai_chat");
        let mut headers = vec![("content-type".to_string(), "application/json".to_string())];
        if let Some(key) = config
            .get("api_key")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
        {
            let (name, value) = match protocol {
                "anthropic_messages" => ("x-api-key", key.to_string()),
                "gemini_api" => ("x-goog-api-key", key.to_string()),
                _ => ("authorization", format!("Bearer {key}")),
            };
            headers.push((name.to_string(), value));
        }
        if protocol == "anthropic_messages" {
            headers.push(("anthropic-version".to_string(), "2023-06-01".to_string()));
        }
        if protocol == "openai_responses" {
            headers.push(("openai-beta".to_string(), "realtime=v1".to_string()));
        }
        validate_trusted_headers(&headers)?;
        let timeout = config
            .get("timeout_secs")
            .and_then(Value::as_u64)
            .unwrap_or(1800);
        redact_secrets(&mut config);
        config["api_base"] = json!(format!(
            "http://127.0.0.1:{BROKER_PORT}{}",
            broker_path(BrokerKind::Model, scope, &id)
        ));
        config["api_key"] = Value::Null;
        let output = destination.join(&id);
        fs::create_dir_all(&output).map_err(|e| invalid(e.to_string()))?;
        fs::write(
            output.join("provider.json"),
            serde_json::to_vec(&config).map_err(|e| invalid(e.to_string()))?,
        )
        .map_err(|e| invalid(e.to_string()))?;
        targets.insert(
            broker_key(BrokerKind::Model, scope, &id),
            BrokerTarget {
                id,
                kind: BrokerKind::Model,
                scope: scope.to_vec(),
                base,
                headers,
                timeout_secs: timeout,
            },
        );
    }
    Ok(())
}

fn project_mcp(
    source: &Path,
    target: &Path,
    scope: &[String],
    targets: &mut HashMap<String, BrokerTarget>,
) -> Result<(), WorkerError> {
    if !source.is_dir() {
        return Ok(());
    }
    let destination = target.join("mcp");
    fs::create_dir_all(&destination).map_err(|e| invalid(e.to_string()))?;
    let manifest = source.join("PLUGIN.yml");
    if regular_file(&manifest) {
        project_plugin_manifest(&manifest, &destination.join("PLUGIN.yml"))?;
    }
    for entry in fs::read_dir(source).map_err(|e| invalid(e.to_string()))? {
        let entry = entry.map_err(|e| invalid(e.to_string()))?;
        if !entry
            .file_type()
            .map_err(|e| invalid(e.to_string()))?
            .is_dir()
        {
            continue;
        }
        let id = entry.file_name().to_string_lossy().to_string();
        let file = entry.path().join("server.json");
        if !regular_file(&file) {
            continue;
        }
        let mut config: Value =
            serde_json::from_slice(&fs::read(&file).map_err(|e| invalid(e.to_string()))?)
                .map_err(|e| invalid(format!("server.json 无效: {e}")))?;
        if config
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("stdio")
            != "http"
        {
            redact_secrets(&mut config);
            config["enabled"] = json!(false);
            config
                .as_object_mut()
                .ok_or_else(|| invalid("MCP 配置不是对象"))?
                .remove("env");
            config
                .as_object_mut()
                .ok_or_else(|| invalid("MCP 配置不是对象"))?
                .remove("command");
            config
                .as_object_mut()
                .ok_or_else(|| invalid("MCP 配置不是对象"))?
                .remove("args");
        } else {
            let Some(url) = config.get("url").and_then(Value::as_str) else {
                return Err(invalid(format!("MCP {id} 缺少 url")));
            };
            let base = Url::parse(url).map_err(|e| invalid(format!("MCP {id} URL 无效: {e}")))?;
            if !matches!(base.scheme(), "http" | "https")
                || base.host_str().is_none()
                || !base.username().is_empty()
                || base.password().is_some()
            {
                return Err(invalid(format!("MCP {id} URL 无效")));
            }
            validate_resource_id(&id)?;
            let headers: Vec<(String, String)> = config
                .get("headers")
                .and_then(Value::as_object)
                .map(|headers| {
                    headers
                        .iter()
                        .filter_map(|(k, v)| {
                            v.as_str().map(|v| (k.to_ascii_lowercase(), v.to_string()))
                        })
                        .collect()
                })
                .unwrap_or_default();
            validate_trusted_headers(&headers)?;
            let timeout = config
                .get("timeout_secs")
                .and_then(Value::as_u64)
                .unwrap_or(30);
            redact_secrets(&mut config);
            config["url"] = json!(format!(
                "http://127.0.0.1:{BROKER_PORT}{}",
                broker_path(BrokerKind::Mcp, scope, &id)
            ));
            let object = config
                .as_object_mut()
                .ok_or_else(|| invalid("MCP 配置不是对象"))?;
            object.remove("headers");
            object.remove("env");
            targets.insert(
                broker_key(BrokerKind::Mcp, scope, &id),
                BrokerTarget {
                    id: id.clone(),
                    kind: BrokerKind::Mcp,
                    scope: scope.to_vec(),
                    base,
                    headers,
                    timeout_secs: timeout,
                },
            );
        }
        let output = destination.join(&id);
        fs::create_dir_all(&output).map_err(|e| invalid(e.to_string()))?;
        fs::write(
            output.join("server.json"),
            serde_json::to_vec(&config).map_err(|e| invalid(e.to_string()))?,
        )
        .map_err(|e| invalid(e.to_string()))?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "container.test.rs"]
mod tests;
