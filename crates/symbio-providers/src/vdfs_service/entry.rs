//! 条目寻址与落盘原语（三种集中实现**共用**的同一套磁盘布局）
//!
//! ## 磁盘布局（与旧的 `FileEntityStore` **完全一致**，不做数据迁移）
//!
//! ```text
//! <类别根>/<id>/<manifest>
//! ```
//!
//! 三个实现的区别**不在布局**，而在**访问拓扑**：单文件型只暴露主文件、
//! 目录型还能下钻条目内部、内存型不落盘。因此「换拓扑」不动磁盘，
//! 「换类别」不碰协议。
//!
//! ## 为什么是自由函数而不是又一个 trait
//!
//! 这些操作没有第二种实现需要切换（磁盘就是磁盘），把它们抽成 trait 只会
//! 重新制造「`EntityStore` 抽象 + 一个实现」那层空转。类型化入口都收在
//! `DirVdfs` / `SingleFileVdfs`
//! 自己身上。

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};
use symbio_kernel::vdfs::{VdfsAccess, VdfsError, VdfsNode, VdfsResult};
use tokio::io::AsyncWriteExt as _;
use tokio::sync::{Mutex, OwnedMutexGuard};

type MutationLock = Arc<Mutex<()>>;

/// 进程内按规范类别根共享的变更锁。
///
/// 锁放在自由函数层而非 `SingleFileVdfs` 字段里：插件经常临时重建 store，克隆也不
/// 复制字段身份；只有按 base 查同一张表，才能覆盖重建后的实例。键使用“最近存在
/// 祖先的 canonical 路径 + 缺失尾部”，因此 base 首次创建前后仍命中同一把锁。
fn mutation_lock_key(base: &Path) -> PathBuf {
    fn lexical_absolute(path: &Path) -> PathBuf {
        let mut out = if path.is_absolute() {
            PathBuf::new()
        } else {
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
        };
        for component in path.components() {
            match component {
                Component::CurDir => {}
                Component::ParentDir => {
                    out.pop();
                }
                _ => out.push(component.as_os_str()),
            }
        }
        out
    }

    let absolute = lexical_absolute(base);
    let mut ancestor = absolute.clone();
    let resolved = loop {
        match std::fs::canonicalize(&ancestor) {
            Ok(path) => break path,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && ancestor.pop() => {}
            Err(_) => break absolute.clone(),
        }
    };
    let tail = absolute.strip_prefix(&ancestor).unwrap_or(Path::new(""));
    let key = if tail.as_os_str().is_empty() {
        resolved
    } else {
        resolved.join(tail)
    };
    #[cfg(windows)]
    {
        // base 尚不存在时，大小写不同的重建实例也必须命中同一把锁。
        PathBuf::from(key.to_string_lossy().to_ascii_lowercase())
    }
    #[cfg(not(windows))]
    {
        key
    }
}

pub(crate) fn mutation_lock(base: &Path) -> MutationLock {
    static LOCKS: OnceLock<std::sync::Mutex<HashMap<PathBuf, MutationLock>>> = OnceLock::new();
    let locks = LOCKS.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    let mut locks = locks.lock().unwrap_or_else(|e| e.into_inner());
    locks
        .entry(mutation_lock_key(base))
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

pub(crate) async fn mutation_guard(base: &Path) -> OwnedMutexGuard<()> {
    mutation_lock(base).lock_owned().await
}

pub(crate) fn is_reparse_point(meta: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt as _;
        meta.file_attributes() & 0x0000_0400 != 0
    }
    #[cfg(not(windows))]
    {
        meta.file_type().is_symlink()
    }
}

async fn reject_reparse_point(path: &Path, label: &str) -> VdfsResult<()> {
    match tokio::fs::symlink_metadata(path).await {
        Ok(meta) if is_reparse_point(&meta) => Err(VdfsError::Forbidden(format!(
            "拒绝使用重解析点作为{label}：{}",
            path.display()
        ))),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(VdfsError::internal(format!("检查{label}失败：{e}"))),
    }
}

async fn reject_reparse_components(path: &Path) -> VdfsResult<()> {
    let mut current = path.to_path_buf();
    loop {
        reject_reparse_point(&current, "路径组件").await?;
        let Some(parent) = current.parent() else {
            break;
        };
        current = parent.to_path_buf();
    }
    Ok(())
}

pub(crate) async fn validate_base(base: &Path) -> VdfsResult<()> {
    reject_reparse_components(base).await?;
    match tokio::fs::symlink_metadata(base).await {
        Ok(meta) if meta.is_dir() => Ok(()),
        Ok(_) => Err(VdfsError::invalid(format!(
            "类别根不是普通目录：{}",
            base.display()
        ))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(VdfsError::internal(format!("检查类别根失败：{e}"))),
    }
}

pub(crate) fn sync_parent_best_effort(parent: &Path) {
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

async fn validate_tree_no_reparse(path: &Path, label: &str) -> VdfsResult<()> {
    reject_reparse_components(path).await?;
    let meta = tokio::fs::symlink_metadata(path)
        .await
        .map_err(|e| VdfsError::internal(format!("检查{label}失败：{e}")))?;
    if is_reparse_point(&meta) {
        return Err(VdfsError::Forbidden(format!(
            "拒绝使用重解析点作为{label}：{}",
            path.display()
        )));
    }
    if meta.is_dir() {
        let mut rd = tokio::fs::read_dir(path)
            .await
            .map_err(|e| VdfsError::internal(format!("读取{label}失败：{e}")))?;
        while let Some(entry) = rd
            .next_entry()
            .await
            .map_err(|e| VdfsError::internal(format!("遍历{label}失败：{e}")))?
        {
            Box::pin(validate_tree_no_reparse(&entry.path(), label)).await?;
        }
    } else if !meta.is_file() {
        return Err(VdfsError::Forbidden(format!(
            "拒绝使用非普通文件作为{label}：{}",
            path.display()
        )));
    }
    Ok(())
}

pub(crate) async fn validate_entry_tree(base: &Path, id: &str) -> VdfsResult<PathBuf> {
    validate_base(base).await?;
    let dir = entry_dir(base, id);
    reject_reparse_point(&dir, "条目目录").await?;
    let meta = tokio::fs::symlink_metadata(&dir)
        .await
        .map_err(|e| VdfsError::not_found(format!("未找到条目「{id}」：{e}")))?;
    if !meta.is_dir() {
        return Err(VdfsError::invalid(format!(
            "条目不是普通目录：{}",
            dir.display()
        )));
    }
    validate_tree_no_reparse(&dir, "条目内容").await?;
    Ok(dir)
}

pub(crate) async fn checked_inner_path(base: &Path, id: &str, rel: &str) -> VdfsResult<PathBuf> {
    validate_base(base).await?;
    let dir = entry_dir(base, id);
    reject_reparse_point(&dir, "条目目录").await?;
    let meta = tokio::fs::symlink_metadata(&dir)
        .await
        .map_err(|e| VdfsError::not_found(format!("未找到条目「{id}」：{e}")))?;
    if !meta.is_dir() {
        return Err(VdfsError::invalid(format!(
            "条目不是普通目录：{}",
            dir.display()
        )));
    }
    let mut current = dir;
    for component in Path::new(rel).components() {
        match component {
            Component::Normal(part) => current.push(part),
            _ => {
                return Err(VdfsError::invalid(format!(
                    "条目内路径必须是普通相对路径：{rel}"
                )))
            }
        }
        reject_reparse_point(&current, "条目内路径组件").await?;
    }
    Ok(current)
}

pub(crate) async fn remove_tree_no_follow(path: &Path) -> VdfsResult<()> {
    reject_reparse_components(path).await?;
    let meta = tokio::fs::symlink_metadata(path)
        .await
        .map_err(|e| VdfsError::internal(format!("检查待删除路径失败：{e}")))?;
    if is_reparse_point(&meta) {
        return Err(VdfsError::Forbidden(format!(
            "拒绝删除重解析点：{}",
            path.display()
        )));
    }
    if meta.is_dir() {
        let mut children = Vec::new();
        let mut rd = tokio::fs::read_dir(path)
            .await
            .map_err(|e| VdfsError::internal(format!("读取待删除目录失败：{e}")))?;
        while let Some(entry) = rd
            .next_entry()
            .await
            .map_err(|e| VdfsError::internal(format!("遍历待删除目录失败：{e}")))?
        {
            children.push(entry.path());
        }
        for child in children {
            Box::pin(remove_tree_no_follow(&child)).await?;
        }
        reject_reparse_components(path).await?;
        let current = tokio::fs::symlink_metadata(path)
            .await
            .map_err(|e| VdfsError::internal(format!("复验待删除目录失败：{e}")))?;
        if is_reparse_point(&current) || !current.is_dir() {
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

pub(crate) async fn sync_tree(path: &Path) -> VdfsResult<()> {
    reject_reparse_components(path).await?;
    let meta = tokio::fs::symlink_metadata(path)
        .await
        .map_err(|e| VdfsError::internal(format!("检查暂存文件失败：{e}")))?;
    if is_reparse_point(&meta) {
        return Err(VdfsError::Forbidden(format!(
            "拒绝同步重解析点：{}",
            path.display()
        )));
    }
    if meta.is_dir() {
        let mut children = Vec::new();
        let mut rd = tokio::fs::read_dir(path)
            .await
            .map_err(|e| VdfsError::internal(format!("读取暂存目录失败：{e}")))?;
        while let Some(entry) = rd
            .next_entry()
            .await
            .map_err(|e| VdfsError::internal(format!("遍历暂存目录失败：{e}")))?
        {
            children.push(entry.path());
        }
        for child in children {
            Box::pin(sync_tree(&child)).await?;
        }
        sync_parent_best_effort(path);
    } else if meta.is_file() {
        let file = tokio::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path)
            .await
            .map_err(|e| VdfsError::internal(format!("打开暂存文件失败：{e}")))?;
        file.sync_all()
            .await
            .map_err(|e| VdfsError::internal(format!("同步暂存文件失败：{e}")))?;
    } else {
        return Err(VdfsError::Forbidden(format!(
            "拒绝同步非普通文件：{}",
            path.display()
        )));
    }
    Ok(())
}

fn manifest_path(dir: &Path, manifest_file: &str) -> VdfsResult<PathBuf> {
    let relative = Path::new(manifest_file);
    if relative.is_absolute()
        || relative.components().any(|component| {
            matches!(
                component,
                Component::Prefix(_) | Component::RootDir | Component::ParentDir
            )
        })
    {
        return Err(VdfsError::invalid("条目主文件必须是条目内的相对路径"));
    }
    Ok(dir.join(relative))
}

async fn reject_manifest_reparse_points(dir: &Path, manifest_file: &str) -> VdfsResult<()> {
    let mut current = dir.to_path_buf();
    for component in Path::new(manifest_file).components() {
        if let Component::Normal(part) = component {
            current.push(part);
            reject_reparse_point(&current, "条目主文件路径").await?;
        }
    }
    Ok(())
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
                    Ok(meta) if is_reparse_point(&meta) => return,
                    Ok(_) => {}
                    Err(_) => return,
                }
                current = path.parent();
            }
            let _ = std::fs::remove_file(&self.path); // grep-audit-allow S-002-bonus: 只清理自己的临时文件
        }
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

// 注：早先这里有 `category_dir(category)` = `<homedir>/<category>`，按插件名反推
// 全局落位。homedir 已下沉到 `home` 插件独有，core 不再提供全局根，故删除——
// 插件一律用父插件经 `PLUGIN_DIR` 告知的自己的目录（`PluginDir::at` 显式传入）。

/// 把条目 id 转成安全的磁盘段名
///
/// - `/` `\` `:` `*` `?` `"` `<` `>` `|` 替换为 `_`
/// - 去除前后空白；空串归一为 `_empty_`
/// - 禁止 `.` 与 `..`（否则条目目录会逃逸出类别根）
pub fn safe_segment(id: &str) -> String {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return "_empty_".to_string();
    }
    if trimmed == "." || trimmed == ".." {
        return format!("_{}_", trimmed);
    }
    let mut s = trimmed.replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|'], "_");
    s = s
        .chars()
        .map(|c| if c.is_control() { '_' } else { c })
        .collect();
    s
}

/// 条目目录：`<base>/<safe(id)>`
pub fn entry_dir(base: &Path, id: &str) -> PathBuf {
    base.join(safe_segment(id))
}

/// 相对路径 → `(条目 id 段, 条目内剩余路径)`
///
/// provider 只见子树内相对路径（规范 §2.4），首段即条目；剩余段用于目录型的
/// 下钻。`""` → `None`（挂载根，不是任何条目）。
pub fn split_rel(path: &str) -> Option<(&str, &str)> {
    let p = path.trim_matches('/');
    if p.is_empty() {
        return None;
    }
    Some(match p.split_once('/') {
        Some((head, rest)) => (head, rest.trim_matches('/')),
        None => (p, ""),
    })
}

/// 路径末段 → 条目 id（去掉 `.<kind>` 呈现扩展名）
///
/// `<根>/skill/demo.skill` 与裸 `demo` 同解——呈现扩展名是**前端选渲染器**的
/// 键，不是地址的一部分，因此寻址必须先剥掉。历史上这段代码在每个资源插件里
/// 各有一份，收敛到这里。
pub fn id_of(path: &str, kind: &str) -> String {
    let base = path.rsplit('/').next().unwrap_or(path);
    base.strip_suffix(&format!(".{kind}"))
        .unwrap_or(base)
        .to_string()
}

/// 整包导入的**建议名**：末段再去掉 `.zip`（新建地址是 `<name>.zip`）
pub fn pack_name_of(path: &str, kind: &str) -> String {
    let base = id_of(path, kind);
    match base.strip_suffix(".zip") {
        Some(stem) if !stem.is_empty() => stem.to_string(),
        _ => base,
    }
}

/// **无名字新建**时的自动条目 id —— 使用方只说「建在这个目录」，名字由插件定。
///
/// 形状 = `<kind>-<8 位十六进制>`：类别前缀让人读得懂磁盘目录名（目录名即 id），
/// 随机段保证唯一——同一个目录里并发新建两项不会撞名。
///
/// 它与 [`id_of`] 是同一件事的两半：**有名字**时 id 来自地址（使用方给），
/// **没名字**时 id 由这里生成（provider 给）。两者都只产出「条目 id」，
/// 谁生成的对外不可见——地址里的 id 永远由 provider 决定。
pub fn auto_id(kind: &str) -> String {
    let mut u = uuid::Uuid::new_v4().simple().to_string();
    u.truncate(8);
    format!("{kind}-{u}")
}

/// 条目摘要（落盘原语向上传递的**唯一**素材形状）
///
/// 「标题 / 状态 / 呈现扩展名 / schema」是各资源的差异，本层给不出，也不该猜；
/// 本层只保证把**寻址、原文、时间戳、字节数**这四样取准，其余由调用方合成节点。
#[derive(Debug, Clone)]
pub struct Entry {
    /// 条目 id（= 磁盘目录名，已安全化）
    pub id: String,
    /// 主文件原文；`None` = 主文件缺失或读失败（条目**存在**但内容不可用）
    pub raw: Option<String>,
    /// 主文件更新时间（Unix 秒）
    pub updated_at: Option<i64>,
    /// 主文件字节数
    pub size: Option<u64>,
}

impl Entry {
    /// 缺省节点：以 id 为名字与标题，`ext` 由主文件名推导
    ///
    /// 无差异的挂载点（不需要自定义标题 / 状态 / schema）直接用它即可，
    /// 这就是「单文件型 / 目录型可以原样注册为挂载点」的落点。
    pub fn node(&self, manifest_file: &str) -> VdfsNode {
        let mut n = VdfsNode::file(self.id.clone(), self.id.clone(), VdfsAccess::READ_WRITE);
        n.ext = manifest_file
            .rsplit_once('.')
            .map(|(_, e)| e.to_ascii_lowercase());
        n.updated_at = self.updated_at;
        n.size = self.size;
        n
    }
}

/// 条目清单：类别根下的**一层目录**，按名升序
///
/// 目录不存在 = 清单为空（首次使用某类别是正常状态，不是错误）。
pub async fn list_entry_ids(base: &Path) -> VdfsResult<Vec<String>> {
    validate_base(base).await?;
    let mut rd = match tokio::fs::read_dir(base).await {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(VdfsError::internal(format!("读取条目清单失败：{e}"))),
    };
    let mut ids = Vec::new();
    while let Some(entry) = rd
        .next_entry()
        .await
        .map_err(|e| VdfsError::internal(format!("遍历条目清单失败：{e}")))?
    {
        let file_type = entry
            .file_type()
            .await
            .map_err(|e| VdfsError::internal(format!("读取条目类型失败：{e}")))?;
        if !file_type.is_dir() {
            continue;
        }
        if file_type.is_symlink() {
            return Err(VdfsError::Forbidden(format!(
                "拒绝使用符号链接作为条目目录：{}",
                entry.path().display()
            )));
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt as _;
            let meta = tokio::fs::symlink_metadata(entry.path())
                .await
                .map_err(|e| VdfsError::internal(format!("读取条目元数据失败：{e}")))?;
            if meta.file_attributes() & 0x0000_0400 != 0 {
                return Err(VdfsError::Forbidden(format!(
                    "拒绝使用重解析点作为条目目录：{}",
                    entry.path().display()
                )));
            }
        }
        // 目录名即 id：非 UTF-8 的名称无法在地址空间里表达，跳过
        if let Some(name) = entry.file_name().to_str() {
            ids.push(name.to_string());
        }
    }
    ids.sort();
    Ok(ids)
}

/// 读条目主文件原文（不存在 → [`VdfsError::NotFound`]）
pub async fn read_entry(base: &Path, id: &str, manifest_file: &str) -> VdfsResult<Entry> {
    // 导入会先删旧目录再逐文件解包；同进程读取等待变更完成，避免 reload 撞上半包。
    let _guard = mutation_guard(base).await;
    read_entry_locked(base, id, manifest_file).await
}

pub(crate) async fn read_entry_locked(
    base: &Path,
    id: &str,
    manifest_file: &str,
) -> VdfsResult<Entry> {
    validate_base(base).await?;
    let dir = entry_dir(base, id);
    reject_reparse_point(&dir, "条目目录").await?;
    let path = manifest_path(&dir, manifest_file)?;
    reject_manifest_reparse_points(&dir, manifest_file).await?;
    validate_base(base).await?;
    reject_manifest_reparse_points(&dir, manifest_file).await?;
    let meta = match tokio::fs::metadata(&path).await {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(VdfsError::NotFound(format!("未找到条目「{id}」：{path:?}")))
        }
        Err(e) => return Err(VdfsError::internal(format!("读取条目失败：{e}"))),
    };
    let text = tokio::fs::read_to_string(&path)
        .await
        .map_err(|e| VdfsError::internal(format!("读取条目内容失败（非文本？）：{e}")))?;
    Ok(Entry {
        id: dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(id)
            .to_string(),
        updated_at: meta.modified().ok().and_then(to_unix),
        size: Some(meta.len()),
        raw: Some(text),
    })
}

/// 读条目主文件原文，**读失败降级为 `None`**（列表用的宽松版）
///
/// 列表不得被单个坏条目挡住：损坏条目降级呈现，而不是让整次 `list` 失败。
pub async fn read_entry_tolerant(base: &Path, id: &str, manifest_file: &str) -> Entry {
    let _guard = mutation_guard(base).await;
    read_entry_tolerant_locked(base, id, manifest_file).await
}

pub(crate) async fn read_entry_tolerant_locked(
    base: &Path,
    id: &str,
    manifest_file: &str,
) -> Entry {
    match read_entry_locked(base, id, manifest_file).await {
        Ok(e) => e,
        Err(_) => Entry {
            id: safe_segment(id),
            raw: None,
            updated_at: stat_mtime(&entry_dir(base, id)).await,
            size: None,
        },
    }
}

/// 写条目主文件（唯一临时文件 + flush/sync + 原子 rename），返回**是否为新建**
///
/// 自由函数入口自己加锁，直接调用者也能与 `SingleFileVdfs` / `DirVdfs` 的变更串行。
pub async fn write_entry(
    base: &Path,
    id: &str,
    manifest_file: &str,
    text: &str,
) -> VdfsResult<bool> {
    let _guard = mutation_guard(base).await;
    write_entry_locked(base, id, manifest_file, text).await
}

/// 调用方已持有同一 base 的 [`mutation_lock`]。
pub(crate) async fn write_entry_locked(
    base: &Path,
    id: &str,
    manifest_file: &str,
    text: &str,
) -> VdfsResult<bool> {
    validate_base(base).await?;
    let dir = entry_dir(base, id);
    reject_reparse_point(&dir, "条目目录").await?;
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| VdfsError::internal(format!("创建条目目录失败：{e}")))?;
    validate_base(base).await?;
    reject_reparse_point(&dir, "条目目录").await?;

    let final_path = manifest_path(&dir, manifest_file)?;
    reject_manifest_reparse_points(&dir, manifest_file).await?;
    if let Some(parent) = final_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| VdfsError::internal(format!("创建主文件目录失败：{e}")))?;
    }
    validate_base(base).await?;
    reject_manifest_reparse_points(&dir, manifest_file).await?;
    write_file_atomic(&final_path, text, "主文件").await
}

pub(crate) async fn write_file_atomic(
    final_path: &Path,
    text: &str,
    label: &str,
) -> VdfsResult<bool> {
    reject_reparse_components(final_path).await?;
    let tmp_path = {
        let mut name = final_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file")
            .to_string();
        name.push('.');
        name.push_str(&uuid::Uuid::new_v4().simple().to_string());
        name.push_str(".tmp");
        final_path.with_file_name(name)
    };

    let (created, previous_permissions) = match tokio::fs::symlink_metadata(&final_path).await {
        Ok(meta) if meta.is_file() => (false, Some(meta.permissions())),
        Ok(meta) if is_reparse_point(&meta) => {
            return Err(VdfsError::Forbidden(format!(
                "拒绝使用重解析点作为{label}：{}",
                final_path.display()
            )))
        }
        Ok(_) => {
            return Err(VdfsError::invalid(format!(
                "{label}目标不是普通文件：{}",
                final_path.display()
            )))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => (true, None),
        Err(e) => return Err(VdfsError::internal(format!("读取目标元数据失败：{e}"))),
    };
    let parent = final_path.parent().ok_or_else(|| {
        VdfsError::invalid(format!("{label}路径没有父目录：{}", final_path.display()))
    })?;
    let mut temp_guard = TempFileGuard::new(tmp_path.clone());
    // create_new 确保即便发生极小概率 UUID 碰撞，也绝不覆盖别人的临时文件。
    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp_path)
        .await
        .map_err(|e| VdfsError::internal(format!("创建临时文件失败：{e}")))?;
    temp_guard.arm();
    file.write_all(text.as_bytes())
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
    reject_reparse_components(final_path).await?;
    if let Some(permissions) = previous_permissions {
        preserve_file_permissions(final_path, &tmp_path, permissions).await?;
    }
    reject_reparse_components(final_path).await?;
    let current = tokio::fs::symlink_metadata(&final_path).await;
    if let Ok(meta) = current {
        if is_reparse_point(&meta) || !meta.is_file() {
            return Err(VdfsError::Forbidden(format!(
                "{label}目标在写入过程中被替换：{}",
                final_path.display()
            )));
        }
    }
    tokio::fs::rename(&tmp_path, &final_path)
        .await
        .map_err(|e| VdfsError::internal(format!("落盘失败：{e}")))?;
    temp_guard.disarm();
    sync_parent_best_effort(parent);
    Ok(created)
}

/// 删除条目目录（不存在 → [`VdfsError::NotFound`]，交由调用方决定是否幂等）
pub async fn remove_entry(base: &Path, id: &str) -> VdfsResult<()> {
    let _guard = mutation_guard(base).await;
    remove_entry_locked(base, id).await
}

/// 调用方已持有同一 base 的 [`mutation_lock`]。
pub(crate) async fn remove_entry_locked(base: &Path, id: &str) -> VdfsResult<()> {
    validate_base(base).await?;
    let dir = entry_dir(base, id);
    reject_reparse_point(&dir, "条目目录").await?;
    let meta = tokio::fs::symlink_metadata(&dir)
        .await
        .map_err(|e| VdfsError::NotFound(format!("磁盘上已无条目目录「{id}」：{e}")))?;
    if !meta.is_dir() {
        return Err(VdfsError::NotFound(format!("磁盘上已无条目目录「{id}」")));
    }
    validate_tree_no_reparse(&dir, "条目内容").await?;
    validate_base(base).await?;
    reject_reparse_point(&dir, "条目目录").await?;
    remove_tree_no_follow(&dir).await?;
    sync_parent_best_effort(base);
    Ok(())
}

/// 条目目录是否存在
pub fn entry_exists(base: &Path, id: &str) -> bool {
    fn ordinary_path(path: &Path) -> bool {
        let mut current = path.to_path_buf();
        loop {
            let Ok(meta) = std::fs::symlink_metadata(&current) else {
                return false;
            };
            if is_reparse_point(&meta) {
                return false;
            }
            let Some(parent) = current.parent() else {
                return true;
            };
            if parent.as_os_str().is_empty() || current == parent {
                return true;
            }
            current = parent.to_path_buf();
        }
    }
    ordinary_path(base)
        && ordinary_path(&entry_dir(base, id))
        && std::fs::symlink_metadata(entry_dir(base, id))
            .map(|meta| meta.is_dir())
            .unwrap_or(false)
}

/// 目录 mtime（拿不到就 `None`——时间戳是展示信息，不值得为此失败）
async fn stat_mtime(path: &Path) -> Option<i64> {
    tokio::fs::metadata(path)
        .await
        .ok()?
        .modified()
        .ok()
        .and_then(to_unix)
}

/// `SystemTime` → Unix 秒
fn to_unix(t: SystemTime) -> Option<i64> {
    t.duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs() as i64)
}

/// 文件 / 目录元数据 → 节点（目录型下钻时呈现真实条目内部）
pub async fn tree_node(path: &Path, name: &str) -> VdfsResult<VdfsNode> {
    let meta = tokio::fs::symlink_metadata(path)
        .await
        .map_err(|e| VdfsError::not_found(format!("无法读取元数据 {path:?}：{e}")))?;
    if is_reparse_point(&meta) {
        return Err(VdfsError::Forbidden(format!(
            "拒绝呈现链接或重解析点：{}",
            path.display()
        )));
    }
    let mut n = if meta.is_dir() {
        VdfsNode::dir(name, name, VdfsAccess::dir(true, true))
    } else {
        VdfsNode::file(name, name, VdfsAccess::file(true))
    };
    n.updated_at = meta.modified().ok().and_then(to_unix);
    if !meta.is_dir() {
        n.size = Some(meta.len());
    }
    Ok(n)
}

#[cfg(test)]
#[path = "entry.test.rs"]
mod tests;
