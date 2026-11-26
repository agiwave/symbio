//! 物理文件层 —— VDFS 的「真实磁盘」那一半
//!
//! ## 为什么在 vdfs 插件里
//!
//! VDFS 整体负责**一棵地址空间**：`.vdfsv2` 打头的是虚拟目录（系统资源类别），
//! 其余地址一律是磁盘上的真实文件。既然两类地址由同一个门面分发，物理文件
//! 访问自然也属于本插件——它不是「挂载点」，没有名字、不参与 `.vdfsv2` 的目录合成。
//!
//! （此前它住在 `crates/symbio-plugin-local`，以 `local` 挂载点身份注册，于是每次本地文件访问
//! 都要先被翻译成 `local/<相对路径>`、再由分发层拆回挂载名与相对路径。那层翻译
//! 除了让 `local` 挤进资源类别清单、继而催生 `nav_visible` 之外，不产生任何价值。）
//!
//! ## 地址规则
//!
//! 进入本层的地址已是「确认非虚拟」的那一半，坐标系是**会话工作目录**
//! （[`VDFS_PARAM_WORKDIR`]，由访问层透传；缺失即 `Internal`——那是接线错误）：
//!
//! - `""` / `"."` = 工作目录根；
//! - `README.md`、`src/main.rs` = 工作目录相对；
//! - `/docs`、`//srv/share/x` = 文件系统绝对（首部 `/` 是锚点，POSIX 根 / Windows 盘根）；
//! - `<盘符>:/tmp/a.txt` = 带盘符的绝对路径。
//!
//! 四条都由 [`Path`] 自己的语义解析，本层不加第二条规则。
//!
//! ## 安全规则（自持，不经机制）
//!
//! 见 [`FsPolicy`]：路径黑名单 + `..` 穿越拒绝 + 符号链接复验，另有读大小 /
//! 列目录条数上限。这些是**物理层专有**的规则，虚拟层没有对应物。

use async_trait::async_trait;
use base64::Engine as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex as StdMutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};
use symbio_plugin_api::{
    vdfs_has_parent_segment, vdfs_path_within, VdfsAccess, VdfsContent, VdfsContext, VdfsError,
    VdfsNode, VdfsProvider, VdfsResult, VdfsValidationError, VdfsWriteResponse, VDFS_PARAM_WORKDIR,
};
use symbio_plugin_api::{VdfsRequest, VdfsResponse};
use tokio::io::AsyncWriteExt as _;
use tokio::sync::Mutex;

/// 单文件读取上限（10MB）
const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024;

/// 单次列目录上限（防止超大目录撑爆上下文）
const MAX_LIST_ENTRIES: usize = 2000;

/// 同进程的物理读写共用一把锁；插件实例可能被访问层反复重建。
fn io_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn unique_path(parent: &Path, tag: &str) -> PathBuf {
    static NEXT: OnceLock<StdMutex<u64>> = OnceLock::new();
    let serial = NEXT
        .get_or_init(|| StdMutex::new(0))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    let mut serial = *serial;
    let nonce = std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    loop {
        serial = serial.wrapping_add(1);
        let path = parent.join(format!(
            ".symbio-{tag}-{}-{nonce}-{serial}.tmp",
            std::process::id()
        ));
        if !path.exists() {
            return path;
        }
    }
}

// ==================== 路径守卫 ====================

/// 规范化路径用于比较：剥掉 Windows 的设备路径前缀，并统一大小写与分隔符。
///
/// 只用于策略比较，不拿它构造实际 I/O 路径。UNC 的 `\\?\UNC\...` 形态必须还原成
/// `\\...`，否则会与普通 UNC 工作区判成两棵不同的树。
fn normalize_for_comparison(path: &Path) -> PathBuf {
    let s = path.to_string_lossy();
    #[cfg(windows)]
    {
        let s = s.replace('/', r"\");
        let stripped = s
            .strip_prefix(r"\\?\")
            .or_else(|| s.strip_prefix(r"\\.\"))
            .unwrap_or(&s);
        let stripped = if stripped.len() >= 4 && stripped[..4].eq_ignore_ascii_case("UNC\\") {
            format!(r"\\{}", &stripped[4..])
        } else {
            stripped.to_string()
        };
        PathBuf::from(stripped.to_ascii_lowercase())
    }
    #[cfg(not(windows))]
    {
        PathBuf::from(s.into_owned())
    }
}

fn starts_with_normalized(base: &Path, prefix: &Path) -> bool {
    normalize_for_comparison(base).starts_with(normalize_for_comparison(prefix))
}

/// 物理文件层的访问策略。
///
/// 只有两类规则：`forbidden_paths` 黑名单，以及可选的「限定在工作区 /
/// 白名单根之内」（[`FsPolicy::workspace_only`]，缺省关闭 = 全盘可读，
/// 仅黑名单兜底）。
///
/// 黑名单按**路径段**比，认的正是 `/` 打头与 `~` 打头的形态：`/etc/passwd` 命中
/// `/etc`、`~/secrets` 命中 `~/…`。绝对锚点地址（见 [`join_target`]）能走到这里，
/// 所以它是最后一道防线——`workspace_only` 打开时连锚点地址也要落进工作区或白名单根。
#[derive(Debug, Clone)]
pub struct FsPolicy {
    /// 是否只允许工作目录与 [`FsPolicy::allowed_roots`] 之内的绝对路径
    pub workspace_only: bool,
    /// 额外的允许根（`workspace_only` 为真时参与判定）
    pub allowed_roots: Vec<PathBuf>,
    /// 黑名单前缀（支持 `~`，命中即拒绝读与写）
    pub forbidden_paths: Vec<String>,
}

impl Default for FsPolicy {
    fn default() -> Self {
        Self {
            workspace_only: false,
            allowed_roots: Vec::new(),
            forbidden_paths: vec![
                "/etc".into(),
                "/root".into(),
                "/usr".into(),
                "~/.ssh".into(),
                "~/.gnupg".into(),
                "~/.aws".into(),
            ],
        }
    }
}

impl FsPolicy {
    /// 两道守卫都走机制层的地址规则（[`vdfs_has_parent_segment`] / [`vdfs_path_within`]），
    /// 不在此另写一份——访问层与物理层必须同一条规则，否则修了一处漏另一处。
    fn path_allowed(&self, path: &Path, workspace_dir: &Path) -> bool {
        let s = normalize_for_comparison(path)
            .to_string_lossy()
            .into_owned();
        if vdfs_has_parent_segment(&s) {
            return false;
        }
        for forbidden in &self.forbidden_paths {
            let expanded = shellexpand::tilde(forbidden);
            let forbidden = normalize_for_comparison(Path::new(expanded.as_ref()))
                .to_string_lossy()
                .into_owned();
            if vdfs_path_within(&s, &forbidden) {
                return false;
            }
        }
        if !self.workspace_only || !path.is_absolute() {
            return true;
        }
        starts_with_normalized(path, workspace_dir)
            || self
                .allowed_roots
                .iter()
                .any(|r| starts_with_normalized(path, r))
    }

    pub fn readable(&self, path: &Path, workspace_dir: &Path) -> bool {
        self.path_allowed(path, workspace_dir)
    }

    pub fn writable(&self, path: &Path, workspace_dir: &Path) -> bool {
        self.path_allowed(path, workspace_dir)
    }
}

// ==================== 地址解析 ====================

/// 工作目录基准
fn workdir(ctx: &VdfsContext) -> VdfsResult<PathBuf> {
    let raw = ctx.param_str(VDFS_PARAM_WORKDIR).ok_or_else(|| {
        VdfsError::internal(format!(
            "缺少 {VDFS_PARAM_WORKDIR} 参数（访问层未透传工作目录）"
        ))
    })?;
    if raw.is_empty() {
        return Err(VdfsError::internal(format!(
            "{VDFS_PARAM_WORKDIR} 参数为空"
        )));
    }
    Ok(PathBuf::from(shellexpand::tilde(raw).into_owned()))
}

/// 地址 → 真实路径。
///
/// **首部的 `/` 是文件系统的绝对锚点，原样交给 [`Path`]**：
/// `docs` 落在 `{workdir}/docs`，`/docs` 落在文件系统根下的 `docs`。相对段经 [`Path::join`]——它
/// 自带操作系统语义：POSIX 下 `join("/docs")` 直接替换成 `/docs`，Windows 下
/// `is_absolute()` 为假但有根，于是带着工作目录的盘符落到 `<盘符>:\docs`。
/// 本层不自造第二条规则：**不要在 join 之前 `trim` 掉首部的 `/`**——那等于把绝对锚点
/// 读成工作目录相对，凭空在 `{workdir}/docs` 造出一棵影子树。
pub(crate) fn join_target(base: &Path, rel: &str) -> PathBuf {
    let p = Path::new(rel);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        base.join(rel)
    }
}

fn to_unix(t: SystemTime) -> Option<i64> {
    t.duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs() as i64)
}

/// 末段名称
fn basename(rel: &str) -> String {
    rel.rsplit('/').next().unwrap_or(rel).to_string()
}

/// 常见图片扩展名 → MIME（多模态读取）
fn image_mime(name: &str) -> Option<&'static str> {
    match Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .as_deref()
    {
        Some("png") => Some("image/png"),
        Some("jpg") | Some("jpeg") => Some("image/jpeg"),
        Some("gif") => Some("image/gif"),
        Some("webp") => Some("image/webp"),
        Some("bmp") => Some("image/bmp"),
        _ => None,
    }
}

/// 目录直接子项计数（`stat` 的 `children` 提示；上限与 [`MAX_LIST_ENTRIES`] 一致）。
///
/// 读不动就返回 `None`（未知）——提示信息不值得让 `stat` 失败。
async fn count_children(dir: &Path) -> Option<u32> {
    let mut rd = tokio::fs::read_dir(dir).await.ok()?;
    let mut n: u32 = 0;
    while let Ok(Some(_)) = rd.next_entry().await {
        n += 1;
        if n as usize >= MAX_LIST_ENTRIES {
            break;
        }
    }
    Some(n)
}

// ==================== 变更路径守卫 ====================

/// Windows 上目录联接也属于重解析点；只查 `is_symlink()` 会漏掉 junction。
fn is_reparse_or_symlink(meta: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt as _;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
        meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        meta.file_type().is_symlink()
    }
}

fn reject_reparse(path: &Path, meta: &std::fs::Metadata) -> VdfsResult<()> {
    if is_reparse_or_symlink(meta) {
        return Err(VdfsError::Forbidden(format!(
            "拒绝访问符号链接或重解析点：{}",
            path.display()
        )));
    }
    Ok(())
}

/// 把相对工作目录变成绝对路径；写入侧需要一个稳定的遍历起点。
async fn absolute_for_mutation(path: &Path) -> VdfsResult<PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    let cwd = std::env::current_dir()
        .map_err(|e| VdfsError::internal(format!("读取当前目录失败：{e}")))?;
    Ok(cwd.join(path))
}

/// 拒绝路径上所有已存在的符号链接 / 重解析点。
///
/// 从目标向根逐级检查 `parent()`：目标不存在时仍会走到最近存在祖先，并能发现
/// 悬空链接本身。必须先查原始路径再 canonicalize，否则 canonicalize 会吞掉链接痕迹。
async fn reject_existing_reparse_components(target: &Path) -> VdfsResult<()> {
    let mut current = target.to_path_buf();
    loop {
        match tokio::fs::symlink_metadata(&current).await {
            Ok(meta) => reject_reparse(&current, &meta)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                return Err(VdfsError::internal(format!(
                    "检查路径组件失败（{}）：{e}",
                    current.display()
                )))
            }
        }
        let Some(parent) = current.parent() else {
            break;
        };
        current = parent.to_path_buf();
    }
    Ok(())
}

async fn validate_base(base: &Path) -> VdfsResult<()> {
    reject_existing_reparse_components(base).await?;
    match tokio::fs::symlink_metadata(base).await {
        Ok(meta) if meta.is_dir() => Ok(()),
        Ok(_) => Err(VdfsError::invalid(format!(
            "工作目录不是普通目录：{}",
            base.display()
        ))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(VdfsError::internal(format!(
            "检查工作目录失败（{}）：{e}",
            base.display()
        ))),
    }
}

/// Windows 没有可移植的目录 fsync；其它平台尽力同步，失败不伪报已持久化。
fn sync_parent_best_effort(parent: &Path) {
    #[cfg(unix)]
    {
        if let Ok(dir) = std::fs::OpenOptions::new().read(true).open(parent) {
            let _ = dir.sync_all();
        }
    }
    #[cfg(not(unix))]
    {
        let _ = parent;
    }
}

#[cfg(not(windows))]
async fn preserve_file_permissions(
    _source: &Path,
    target: &Path,
    permissions: std::fs::Permissions,
) -> VdfsResult<()> {
    tokio::fs::set_permissions(target, permissions)
        .await
        .map_err(|e| VdfsError::internal(format!("保留原文件权限失败：{e}")))
}

#[cfg(windows)]
async fn preserve_file_permissions(
    source: &Path,
    target: &Path,
    permissions: std::fs::Permissions,
) -> VdfsResult<()> {
    tokio::fs::set_permissions(target, permissions)
        .await
        .map_err(|e| VdfsError::internal(format!("保留原文件属性失败：{e}")))?;
    let source = PathBuf::from(source.as_os_str());
    let target = PathBuf::from(target.as_os_str());
    tokio::task::spawn_blocking(move || {
        use std::os::windows::ffi::OsStrExt as _;
        let literal = |path: &Path| {
            path.as_os_str()
                .encode_wide()
                .map(|unit| unit.to_string())
                .collect::<Vec<_>>()
                .join(",")
        };
        let script = format!(
            "$ErrorActionPreference='Stop';$ProgressPreference='SilentlyContinue';\
             $source=-join[char[]]@({});$target=-join[char[]]@({});\
             $acl=Get-Acl -LiteralPath $source;Set-Acl -LiteralPath $target -AclObject $acl",
            literal(&source),
            literal(&target),
        );
        let output = std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .output()
            .map_err(|e| VdfsError::internal(format!("调用 PowerShell 保留 ACL 失败：{e}")))?;
        if output.status.success() {
            Ok(())
        } else {
            Err(VdfsError::internal(format!(
                "保留原文件 ACL 失败：{}",
                String::from_utf8_lossy(&output.stderr).trim()
            )))
        }
    })
    .await
    .map_err(|e| VdfsError::internal(format!("保留原文件 ACL 任务失败：{e}")))?
}

struct TempFileGuard {
    path: PathBuf,
    armed: bool,
}

impl TempFileGuard {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: false }
    }

    fn arm(&mut self) {
        self.armed = true;
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        if self.armed {
            let mut current = self.path.parent();
            while let Some(path) = current {
                match std::fs::symlink_metadata(path) {
                    Ok(meta) if is_reparse_or_symlink(&meta) => return,
                    Ok(_) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
                    Err(_) => return,
                }
                current = path.parent();
            }
            let _ = std::fs::remove_file(&self.path); // grep-audit-allow S-002-bonus: 只清理本次 create_new 创建的路径
        }
    }
}

async fn validate_tree_for_delete(path: &Path) -> VdfsResult<()> {
    reject_existing_reparse_components(path).await?;
    let meta = tokio::fs::symlink_metadata(path)
        .await
        .map_err(|e| VdfsError::not_found(format!("无法访问 {}：{e}", path.display())))?;
    reject_reparse(path, &meta)?;
    if meta.is_dir() {
        let mut rd = tokio::fs::read_dir(path)
            .await
            .map_err(|e| VdfsError::internal(format!("读取目录失败：{e}")))?;
        while let Some(entry) = rd
            .next_entry()
            .await
            .map_err(|e| VdfsError::internal(format!("遍历目录失败：{e}")))?
        {
            Box::pin(validate_tree_for_delete(&entry.path())).await?;
        }
    } else if !meta.is_file() {
        return Err(VdfsError::Forbidden(format!(
            "拒绝删除非普通文件：{}",
            path.display()
        )));
    }
    Ok(())
}

async fn remove_tree_no_follow(path: &Path) -> VdfsResult<()> {
    reject_existing_reparse_components(path).await?;
    let meta = tokio::fs::symlink_metadata(path)
        .await
        .map_err(|e| VdfsError::not_found(format!("无法访问 {}：{e}", path.display())))?;
    reject_reparse(path, &meta)?;
    if meta.is_dir() {
        let mut children = Vec::new();
        let mut rd = tokio::fs::read_dir(path)
            .await
            .map_err(|e| VdfsError::internal(format!("读取目录失败：{e}")))?;
        while let Some(entry) = rd
            .next_entry()
            .await
            .map_err(|e| VdfsError::internal(format!("遍历目录失败：{e}")))?
        {
            children.push(entry.path());
        }
        for child in children {
            Box::pin(remove_tree_no_follow(&child)).await?;
        }
        reject_existing_reparse_components(path).await?;
        let meta = tokio::fs::symlink_metadata(path)
            .await
            .map_err(|e| VdfsError::internal(format!("复验目录失败（{}）：{e}", path.display())))?;
        reject_reparse(path, &meta)?;
        if !meta.is_dir() {
            return Err(VdfsError::Forbidden(format!(
                "目录在删除过程中被替换：{}",
                path.display()
            )));
        }
        tokio::fs::remove_dir(path)
            .await
            .map_err(|e| VdfsError::internal(format!("删除目录失败：{e}")))?;
    } else if meta.is_file() {
        tokio::fs::remove_file(path)
            .await
            .map_err(|e| VdfsError::internal(format!("删除文件失败：{e}")))?;
    } else {
        return Err(VdfsError::Forbidden(format!(
            "拒绝删除非普通文件：{}",
            path.display()
        )));
    }
    Ok(())
}

/// 找到最近的存在祖先；目标本身存在时它就是最近祖先。
async fn nearest_existing_ancestor(target: &Path) -> VdfsResult<PathBuf> {
    let mut current = target.to_path_buf();
    loop {
        match tokio::fs::symlink_metadata(&current).await {
            Ok(_) => return Ok(current),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let Some(parent) = current.parent() else {
                    return Err(VdfsError::not_found(format!(
                        "路径不存在：{}",
                        target.display()
                    )));
                };
                current = parent.to_path_buf();
            }
            Err(e) => {
                return Err(VdfsError::internal(format!(
                    "检查路径失败（{}）：{e}",
                    current.display()
                )))
            }
        }
    }
}

// ==================== 物理文件层 ====================

/// 磁盘上的真实文件树（工作目录为根）。
///
/// 实现 [`VdfsProvider`] 只为复用同一套分发与组合逻辑（`edit` / `search` 由
/// 访问层组合而成，不区分虚拟与物理）；它**不注册为挂载点**，因此
/// `label` / `order` / 根节点的 `new_type` 这些「被合成进 `.vdfsv2` 时才有人读」的
/// 声明在这里没有意义，一律不覆盖。
pub struct PhysicalFs {
    policy: FsPolicy,
}

impl Default for PhysicalFs {
    fn default() -> Self {
        Self::new()
    }
}

impl PhysicalFs {
    pub fn new() -> Self {
        Self {
            policy: FsPolicy::default(),
        }
    }

    /// 读路径守卫：黑名单 + 拒绝链接组件 + 解析后复验。
    ///
    /// 沙箱授权只能看到词法地址；若允许链接，`allowed/link` 可在授权后解析到
    /// `allowed/denied`，绕过路径规则。因此模型读路径不跟随符号链接或重解析点，
    /// canonicalize 后的复验只作为纵深防御。外部进程仍可在校验后替换路径组件；
    /// 进程内锁不协调外部写者，彻底消除该 TOCTOU 需要句柄相对且拒绝链接的 I/O。
    ///
    /// `addr` 是**调用方交来的地址**，错误文案一律以它为主语：物理路径此前直接出现在
    /// 文案里（`目录不可读：<工作目录>\docs`），而大模型把这条诊断当成了地址本身——
    /// 实测会话 cli1a175d2ff0：`vdfs_read {"path":"docs"}` 报「目录不可读」后，下一次
    /// 调用就是 `vdfs_read {"path":"<那条真实路径>"}`，同一错误再吃一次，连撞三轮才换成
    /// `vdfs_list`。**能被复制的错误文案必须是可执行的下一步**，不是一个看起来像地址的
    /// 字符串（物理位置只作上下文放在末尾，并标明它是工作目录）。
    async fn guard_read(&self, base: &Path, target: &Path, addr: &str) -> VdfsResult<PathBuf> {
        validate_base(base).await?;
        if !self.policy.readable(target, base) {
            return Err(VdfsError::Forbidden(format!("路径不允许访问：'{addr}'")));
        }
        reject_existing_reparse_components(target).await?;
        let resolved = tokio::fs::canonicalize(target).await.map_err(|e| {
            VdfsError::not_found(
                format!("路径不存在：'{addr}'（在工作目录 {} 下找）", base.display())
                    + &format!("：{e}"),
            )
        })?;
        if !self.policy.readable(&resolved, base) {
            return Err(VdfsError::Forbidden(format!(
                "路径解析后超出工作区范围：{}",
                resolved.display()
            )));
        }
        Ok(resolved)
    }

    /// 变更路径守卫：黑名单 + 工作区复验 + 拒绝任何已存在的链接组件。
    ///
    /// 不存在的目标会先解析到最近的存在祖先，再把缺失尾部拼回解析后的祖先；这样
    /// `workspace_only` 与黑名单校验不会因为“目标尚不存在”而被跳过。目录创建后
    /// 调用方必须再次调用本守卫。
    ///
    /// 这里仍是路径式 I/O：检查与后续操作之间，外部进程若替换已检查的目录，标准库
    /// 无法用已校验的目录句柄固定后续操作。每次实际 I/O 前都会复验，递归删除还会
    /// 逐项 `lstat` 且不跟随，但不能声称消除 TOCTOU；彻底固定句柄需要 `openat2` /
    /// handle-relative Windows 实现或新增成熟依赖。
    async fn guard_write(&self, base: &Path, target: &Path) -> VdfsResult<PathBuf> {
        validate_base(base).await?;
        if !self.policy.writable(target, base) {
            return Err(VdfsError::Forbidden(format!(
                "路径不允许写入：{}",
                target.display()
            )));
        }
        let base = absolute_for_mutation(base).await?;
        let target = absolute_for_mutation(target).await?;
        reject_existing_reparse_components(&target).await?;

        let ancestor = nearest_existing_ancestor(&target).await?;
        let resolved_ancestor = tokio::fs::canonicalize(&ancestor).await.map_err(|e| {
            VdfsError::internal(format!("解析路径失败（{}）：{e}", ancestor.display()))
        })?;
        if !self.policy.writable(&resolved_ancestor, &base) {
            return Err(VdfsError::Forbidden(format!(
                "路径解析后超出工作区范围：{}",
                resolved_ancestor.display()
            )));
        }

        let tail = target.strip_prefix(&ancestor).unwrap_or(Path::new(""));
        let resolved = if tail.as_os_str().is_empty() {
            resolved_ancestor
        } else {
            resolved_ancestor.join(tail)
        };
        if !self.policy.writable(&resolved, &base) {
            return Err(VdfsError::Forbidden(format!(
                "路径解析后超出工作区范围：{}",
                resolved.display()
            )));
        }
        // canonicalize 后的组件也应复核一次，避免解析结果携带意外重解析点。
        reject_existing_reparse_components(&resolved).await?;
        Ok(resolved)
    }

    /// 目录 / 文件节点形状（虚拟层与物理层共用同一套 `VdfsNode` 构造）
    fn node_from(name: String, is_dir: bool) -> VdfsNode {
        if is_dir {
            VdfsNode::dir(name.clone(), name, VdfsAccess::dir(true, true))
        } else {
            VdfsNode::file(name.clone(), name, VdfsAccess::file(true))
        }
    }
}

/// 唯一入口 `dispatch`：按请求变体落到各私有操作上；地址（`path`）是独立参数，
/// 分发先按 path 定位、再按操作落地。
#[async_trait]
impl VdfsProvider for PhysicalFs {
    async fn dispatch(
        &self,
        ctx: &VdfsContext,
        path: &str,
        req: VdfsRequest,
    ) -> VdfsResult<VdfsResponse> {
        // 原子替换避免 readers 撞上原地截断；同一把进程锁也串行化写、导入式删除与读。
        let _io_guard = io_lock().lock().await;
        match req {
            VdfsRequest::List { .. } => Ok(VdfsResponse::list(self.do_list(ctx, path).await?)),
            VdfsRequest::Stat => Ok(VdfsResponse::Stat(self.do_stat(ctx, path).await?)),
            VdfsRequest::Read => Ok(VdfsResponse::Read(self.do_read(ctx, path).await?)),
            VdfsRequest::Write { content } => Ok(VdfsResponse::Write(
                self.do_write(ctx, path, &content).await?,
            )),
            VdfsRequest::Delete { recursive } => {
                self.do_delete(ctx, path, recursive).await?;
                Ok(VdfsResponse::Unit)
            }
            VdfsRequest::Mkdir => {
                self.do_mkdir(ctx, path).await?;
                Ok(VdfsResponse::Unit)
            }
            _ => Err(VdfsError::NotImplemented),
        }
    }
}

impl PhysicalFs {
    async fn do_list(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<Vec<VdfsNode>> {
        let base = workdir(ctx)?;
        let target = join_target(&base, path);
        let resolved = self.guard_read(&base, &target, path).await?;

        let mut rd = tokio::fs::read_dir(&resolved)
            .await
            .map_err(|e| VdfsError::invalid(format!("无法读取目录 '{path}'：{e}")))?;

        let mut nodes: Vec<VdfsNode> = Vec::new();
        while let Some(entry) = rd
            .next_entry()
            .await
            .map_err(|e| VdfsError::internal(format!("遍历目录失败：{e}")))?
        {
            let Ok(name) = entry.file_name().into_string() else {
                continue; // 非 UTF-8 名称跳过（无法在地址空间里表达）
            };
            let Ok(meta) = entry.metadata().await else {
                continue;
            };
            let is_dir = meta.is_dir();
            let mut node = Self::node_from(name.clone(), is_dir);
            node.updated_at = meta.modified().ok().and_then(to_unix);
            if !is_dir {
                node.size = Some(meta.len());
                node.binary = image_mime(&name).is_some();
            }
            nodes.push(node);
            if nodes.len() >= MAX_LIST_ENTRIES {
                break;
            }
        }

        // 目录在前，各自按名称升序
        nodes.sort_by(|a, b| {
            b.is_dir()
                .cmp(&a.is_dir())
                .then_with(|| a.name.cmp(&b.name))
        });
        Ok(nodes)
    }

    async fn do_stat(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsNode> {
        let base = workdir(ctx)?;
        let target = join_target(&base, path);
        let resolved = self.guard_read(&base, &target, path).await?;
        let meta = tokio::fs::metadata(&resolved)
            .await
            .map_err(|e| VdfsError::not_found(format!("无法读取元数据 '{path}'：{e}")))?;

        let name = if path.is_empty() {
            resolved
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("工作目录")
                .to_string()
        } else {
            basename(path)
        };
        let is_dir = meta.is_dir();
        let mut node = Self::node_from(name.clone(), is_dir);
        node.updated_at = meta.modified().ok().and_then(to_unix);
        if !is_dir {
            node.size = Some(meta.len());
            node.binary = image_mime(&name).is_some();
        } else {
            node.children = count_children(&resolved).await;
        }
        Ok(node)
    }

    async fn do_read(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsContent> {
        let base = workdir(ctx)?;
        let target = join_target(&base, path);
        let resolved = self.guard_read(&base, &target, path).await?;

        let meta = tokio::fs::metadata(&resolved)
            .await
            .map_err(|e| VdfsError::not_found(format!("无法读取文件元数据：{e}")))?;
        if meta.is_dir() {
            // 文案里给出**可直接复制的下一次调用**，而不是只说「请用 vdfs_list」：
            // 实测同一句提示被连撞三次（模型每次改的是**地址写法**，不是**工具**），
            // 因为提示没有告诉它「地址不用改」。
            return Err(VdfsError::Forbidden(format!(
                "'{path}' 是目录，没有正文可读。改调 vdfs_list 列举它的子项，地址原样可用：{{\"path\":\"{path}\"}}"
            )));
        }
        if meta.len() > MAX_FILE_SIZE {
            return Err(VdfsError::Forbidden(format!(
                "文件过大：{} 字节（上限 {} 字节）",
                meta.len(),
                MAX_FILE_SIZE
            )));
        }

        // 图片：base64 载荷（多模态）
        if let Some(mime) = image_mime(path) {
            let bytes = tokio::fs::read(&resolved)
                .await
                .map_err(|e| VdfsError::internal(format!("读取文件失败：{e}")))?;
            let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
            return Ok(VdfsContent::binary(b64, bytes.len() as u64).with_mime(mime));
        }

        let text = tokio::fs::read_to_string(&resolved)
            .await
            .map_err(|e| VdfsError::internal(format!("读取文件失败（非文本？）：{e}")))?;
        Ok(VdfsContent::text(text))
    }

    async fn do_write(
        &self,
        ctx: &VdfsContext,
        path: &str,
        content: &VdfsContent,
    ) -> VdfsResult<VdfsWriteResponse> {
        let base = workdir(ctx)?;
        let target = join_target(&base, path);
        let mut resolved = self.guard_write(&base, &target).await?;

        let bytes: Vec<u8> = match (&content.text, &content.b64) {
            (Some(t), _) => t.as_bytes().to_vec(),
            (None, Some(b)) => base64::engine::general_purpose::STANDARD
                .decode(b)
                .map_err(|e| {
                    VdfsError::Invalid(VdfsValidationError::new(format!("b64 解码失败：{e}")))
                })?,
            (None, None) => {
                return Err(VdfsError::invalid("写入需要 text 或 b64 之一作为内容"));
            }
        };

        if let Some(parent) = resolved.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| VdfsError::internal(format!("创建目录失败：{e}")))?;
        }
        // 新建的父目录此前不存在，创建完成后必须重新解析并复验整条路径。
        resolved = self.guard_write(&base, &target).await?;
        let (created, previous_permissions) = match tokio::fs::symlink_metadata(&resolved).await {
            Ok(meta) if meta.is_file() => (false, Some(meta.permissions())),
            Ok(_) => (false, None),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (true, None),
            Err(e) => return Err(VdfsError::internal(format!("读取目标元数据失败：{e}"))),
        };
        let parent = resolved
            .parent()
            .ok_or_else(|| {
                VdfsError::invalid(format!("文件路径没有父目录：{}", resolved.display()))
            })?
            .to_path_buf();
        let mut tmp = unique_path(&parent, "write");
        let mut file = loop {
            match tokio::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp)
                .await
            {
                Ok(file) => break file,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    tmp = unique_path(&parent, "write");
                }
                Err(e) => return Err(VdfsError::internal(format!("创建临时文件失败：{e}"))),
            }
        };
        let mut temp_guard = TempFileGuard::new(tmp.clone());
        temp_guard.arm();
        file.write_all(&bytes)
            .await
            .map_err(|e| VdfsError::internal(format!("写入临时文件失败：{e}")))?;
        file.flush()
            .await
            .map_err(|e| VdfsError::internal(format!("刷新临时文件失败：{e}")))?;
        file.sync_all()
            .await
            .map_err(|e| VdfsError::internal(format!("同步临时文件失败：{e}")))?;
        // Windows 上 rename 前必须关闭文件句柄。
        drop(file);
        if let Some(permissions) = previous_permissions {
            preserve_file_permissions(&resolved, &tmp, permissions).await?;
        }

        // 写临时文件期间父链可能被换掉；rename 前按原始地址重新解析并复验。
        resolved = self.guard_write(&base, &target).await?;
        let rechecked_parent = resolved.parent().ok_or_else(|| {
            VdfsError::invalid(format!("文件路径没有父目录：{}", resolved.display()))
        })?;
        if rechecked_parent != parent {
            return Err(VdfsError::Forbidden(format!(
                "目标父目录在写入过程中发生变化：{}",
                parent.display()
            )));
        }
        tokio::fs::rename(&tmp, &resolved)
            .await
            .map_err(|e| VdfsError::internal(format!("原子替换文件失败：{e}")))?;
        temp_guard.disarm();
        sync_parent_best_effort(rechecked_parent);

        Ok(VdfsWriteResponse {
            // 具名写：名字是调用方给的（磁盘层没有「匿名新建」这一形态）
            name: None,
            created,
        })
    }

    async fn do_delete(&self, ctx: &VdfsContext, path: &str, recursive: bool) -> VdfsResult<()> {
        let base = workdir(ctx)?;
        let target = join_target(&base, path);
        // 删除不能沿用读守卫：读允许解析后仍在策略内的链接，删除必须拒绝链接本身。
        let resolved = self.guard_write(&base, &target).await?;

        let meta = tokio::fs::symlink_metadata(&resolved)
            .await
            .map_err(|e| VdfsError::not_found(format!("无法访问 {}：{e}", target.display())))?;
        reject_reparse(&resolved, &meta)?;
        if meta.is_dir() {
            if !recursive {
                return Err(VdfsError::invalid(format!(
                    "目录需要 recursive=true 才能删除：{}",
                    target.display()
                )));
            }
            // 先完整预检，发现任何子链接就一项都不删。
            validate_tree_for_delete(&resolved).await?;
            self.guard_write(&base, &target).await?;
            remove_tree_no_follow(&resolved).await?;
        } else if meta.is_file() {
            self.guard_write(&base, &target).await?;
            let current = tokio::fs::symlink_metadata(&resolved)
                .await
                .map_err(|e| VdfsError::internal(format!("复验文件失败：{e}")))?;
            reject_reparse(&resolved, &current)?;
            if !current.is_file() {
                return Err(VdfsError::Forbidden("文件在删除前被替换".to_string()));
            }
            tokio::fs::remove_file(&resolved)
                .await
                .map_err(|e| VdfsError::internal(format!("删除文件失败：{e}")))?;
        } else {
            return Err(VdfsError::Forbidden(format!(
                "拒绝删除非普通文件：{}",
                resolved.display()
            )));
        }
        if let Some(parent) = resolved.parent() {
            sync_parent_best_effort(parent);
        }
        Ok(())
    }

    async fn do_mkdir(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<()> {
        let base = workdir(ctx)?;
        let target = join_target(&base, path);
        let resolved = self.guard_write(&base, &target).await?;
        if resolved.parent().is_none() {
            return Err(VdfsError::Forbidden("不可删除文件系统根目录".to_string()));
        }
        tokio::fs::create_dir_all(&resolved)
            .await
            .map_err(|e| VdfsError::internal(format!("创建目录失败：{e}")))?;
        self.guard_write(&base, &target).await?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "physical.test.rs"]
mod tests;
