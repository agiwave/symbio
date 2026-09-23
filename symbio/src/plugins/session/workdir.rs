//! 会话工作目录的 VDFS 场景实现（`<根>/session/<id>/工作目录/<rel>`）
//!
//! 本模块把**会话工作目录**的文件系统层级表达为 VDFS 节点：目录 → 只读
//! （`l`，可下钻），文件 → 可读写（`rw`）。列 / 读 / 写 / 删四个函数是
//! `SessionPlugin` 的 `VdfsProvider` 实现在「工作目录」这一段上的落点——
//! **不存在第二套树协议**。
//!
//! 与磁盘上的物理层（`plugins/vdfs/physical.rs`）不同，这里的地址是**会话内**
//! 的（工作目录由会话元数据决定），因此归 session 插件，而不是 vdfs 插件。
//!
//! 路径安全：节点地址一律为工作目录内的相对路径（`/` 分隔）；拒绝 `..`
//! 与绝对路径，并在 join 后做前缀校验（双重闸门，与 agent 目录的
//! 路径白名单同风格）。

use crate::symbio_core::vdfs::{ChangeSubscriptions, VdfsChange};
use crate::symbio_core::vdfs_provider::{VdfsAccess, VdfsNode};
use crate::symbio_core::PluginError;
use dashmap::DashMap;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::fs_watcher::FsWatcher;
use super::types::Session;

/// 工作目录树节点的 kind（场景自定；机制层仅透传，不参与能力判定）
pub const TREE_KIND: &str = "dir";

/// 子类别的项级图标分发键（前端 `registry/vdfsIcons` 按 `config_type` 取图）
const ATTR_CONFIG_TYPE: &str = "config_type";

/// 会话内部：子会话清单的路径段（同时是展示名）
pub const SEG_SUB_SESSIONS: &str = "子会话";
/// 会话内部：工作目录树的路径段（同时是展示名）
pub const SEG_WORKDIR: &str = "工作目录";

/// 从会话元数据取工作目录（缺失/为空 = 该会话无 tree 数据）
pub fn workdir_of(session: &Session) -> Option<String> {
    session
        .metadata
        .get("workdir")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// 校验并拼接工作目录内相对路径（拒绝穿越/绝对路径，返回 (绝对路径, 规范相对路径)）
fn resolve_under(workdir: &str, rel: &str) -> Result<(PathBuf, String), PluginError> {
    let normalized = rel.trim_matches('/');
    if normalized.contains('\\') || normalized.split('/').any(|seg| seg == "..") {
        return Err(PluginError::ValidationError(format!(
            "非法路径（拒绝路径穿越）: {rel}"
        )));
    }
    let abs = Path::new(workdir).join(normalized);
    // 前缀校验（路径穿越之外的第二道安全网；两侧同为规范化形式）
    let base = canonicalize_loose(Path::new(workdir));
    if !canonicalize_loose(&abs).starts_with(&base) {
        return Err(PluginError::ValidationError(format!(
            "非法路径（越出工作目录）: {rel}"
        )));
    }
    Ok((abs, normalized.to_string()))
}

/// 规范化路径：目标**尚不存在**时 `canonicalize` 会失败（Windows 上它还返回
/// `\\?\` 前缀形式），直接拿去比较会把「工作目录内新建文件」误判为越界。
///
/// 故逐级回退到**最近的存在祖先**做规范化，再接回剩余段：既保留对符号链接
/// 穿越的校验，也不误伤合法的新建 / 删除路径。
fn canonicalize_loose(p: &Path) -> PathBuf {
    let mut anchor = p.to_path_buf();
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    while anchor.canonicalize().is_err() {
        match anchor.file_name() {
            Some(name) => {
                tail.push(name.to_os_string());
                anchor.pop();
            }
            None => break,
        }
    }
    let mut resolved = anchor.canonicalize().unwrap_or_else(|_| anchor.clone());
    for name in tail.into_iter().rev() {
        resolved.push(name);
    }
    resolved
}

/// 列出 `parent`（相对路径，`None` = 根层）的下一层节点。
///
/// 节点形状完全由访问位表达：目录 → `l`（可下钻），文件 → `rw`（可编辑）。
/// `name` 是**本层段名**（不是全相对路径——全路径由分发层按请求路径回填），
/// `config_type` 是项级图标分发键；隐藏项（`.` 开头）不下发。
/// 排序：目录优先、名称字典序（与前端树展示约定一致）。
///
/// 与 `read_node` / `write_node` / `delete_node` 同形：**不依赖请求 ctx**。
pub async fn list_children(
    workdir: &str,
    parent: Option<&str>,
) -> Result<Vec<VdfsNode>, PluginError> {
    let parent_rel = parent.unwrap_or("").trim_matches('/').to_string();
    let (dir_abs, _) = resolve_under(workdir, &parent_rel)?;
    if !dir_abs.is_dir() {
        return Err(PluginError::NotFound(format!("目录不存在: {parent_rel}")));
    }

    let mut entries = tokio::fs::read_dir(&dir_abs)
        .await
        .map_err(|e| PluginError::InternalError(format!("读取目录失败: {e}")))?;

    let mut nodes: Vec<VdfsNode> = Vec::new();
    while let Some(entry) = entries
        .next_entry()
        .await
        .map_err(|e| PluginError::InternalError(e.to_string()))?
    {
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue; // 隐藏项不下发
        }
        let meta = entry
            .metadata()
            .await
            .map_err(|e| PluginError::InternalError(e.to_string()))?;
        nodes.push(tree_node(&name, &meta));
    }

    // 目录优先，其余按名称字典序（判定只看访问位，不看 kind）
    nodes.sort_by(|a, b| {
        (!a.is_dir())
            .cmp(&!b.is_dir())
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(nodes)
}

/// 文件 / 目录元数据 → 节点（列与 stat 共用同一份形状，两条链路不会分叉）
fn tree_node(name: &str, meta: &std::fs::Metadata) -> VdfsNode {
    let is_dir = meta.is_dir();
    let mut n = if is_dir {
        VdfsNode::dir(name, name, VdfsAccess::LIST)
    } else {
        let mut f = VdfsNode::file(name, name, VdfsAccess::READ_WRITE);
        f.size = Some(meta.len());
        f
    };
    n.kind = TREE_KIND.to_string();
    n.updated_at = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64);
    n.attributes.insert(
        ATTR_CONFIG_TYPE.to_string(),
        json!(if is_dir { "directory" } else { "file" }),
    );
    n
}

/// 读取单个树节点的元数据（目录 → 只读；文件 → 可读写 + 字节数）。
///
/// **不内联正文**：内容语义只在 `vdfs/read` 上（见 [`read_content`]），
/// `stat` 只回答「这是个什么节点」。
pub async fn read_node(workdir: &str, rel: &str) -> Result<VdfsNode, PluginError> {
    let (abs, rel_norm) = resolve_under(workdir, rel)?;
    let meta = tokio::fs::metadata(&abs)
        .await
        .map_err(|_| PluginError::NotFound(format!("路径不存在: {rel}")))?;
    let name = abs
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| rel_norm.rsplit('/').next().unwrap_or(&rel_norm).to_string());
    Ok(tree_node(&name, &meta))
}

/// 读取文件内容（UTF-8）。目录 / 不存在 / 不可解码均报错。
pub async fn read_content(workdir: &str, rel: &str) -> Result<String, PluginError> {
    let (abs, _) = resolve_under(workdir, rel)?;
    let meta = tokio::fs::metadata(&abs)
        .await
        .map_err(|_| PluginError::NotFound(format!("路径不存在: {rel}")))?;
    if meta.is_dir() {
        return Err(PluginError::ValidationError(format!(
            "是目录，不能读取内容: {rel}"
        )));
    }
    tokio::fs::read_to_string(&abs)
        .await
        .map_err(|e| PluginError::InternalError(format!("读取失败（非 UTF-8 或不可读）: {e}")))
}

/// 写入文件节点（目录树文件编辑的写回；不存在则创建，目标是目录时拒绝）
pub async fn write_node(workdir: &str, rel: &str, content: &str) -> Result<(), PluginError> {
    let (abs, _) = resolve_under(workdir, rel)?;
    if abs.is_dir() {
        return Err(PluginError::ValidationError(format!(
            "目标是目录，不能作为文件写入: {rel}"
        )));
    }
    if let Some(parent) = abs.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| PluginError::InternalError(e.to_string()))?;
    }
    tokio::fs::write(&abs, content)
        .await
        .map_err(|e| PluginError::InternalError(e.to_string()))
}

/// 删除文件节点（仅文件；目录删除不属于本场景能力）
pub async fn delete_node(workdir: &str, rel: &str) -> Result<(), PluginError> {
    let (abs, _) = resolve_under(workdir, rel)?;
    let meta = tokio::fs::metadata(&abs)
        .await
        .map_err(|_| PluginError::NotFound(format!("路径不存在: {rel}")))?;
    if meta.is_dir() {
        return Err(PluginError::ValidationError(format!(
            "仅支持删除文件，目录: {rel}"
        )));
    }
    tokio::fs::remove_file(&abs)
        .await
        .map_err(|e| PluginError::InternalError(e.to_string()))
}

/// 工作目录监听管理器（场景级基础设施）。
///
/// 监听生命周期与**前端树视图的挂载期**绑定（`vdfs/watch` /
/// `vdfs/unwatch` 操作对）：每个 workdir 至多一个 [`FsWatcher`]，以
/// 关注它的容器（会话）引用计数维持——不同会话的工作目录各自监听，共享
/// 工作目录的多个会话共享同一监听，最后一个订阅方释放后监听停止。
/// 文件变化时把容器 id 翻译成 VDFS 路径，投进会话订阅者共用的那张变更表
/// （[`ChangeSubscriptions::notify`]），驱动前端树视图与详情编辑器按 §3.3
/// 防抖重载。机制层只约定「这个地址变了」语义，不感知文件细节。
///
/// **释放采用世代守卫的延迟释放**：unwatch 经网络在途，可能在快速
/// 「离开→返回」后晚于新一轮 watch 到达；直接释放会误杀刚重建的订阅
/// （树失去实时更新）。释放任务延迟一个宽限期执行，且仅当 (workdir,
/// container) 的世代号未变（期间无重新订阅）才真正释放。
#[derive(Default)]
pub struct WorkdirWatchManager {
    /// workdir → 持活的监听器（保持句柄以维持监听）
    ///
    /// 注意：字段必须为 `Arc<DashMap>`——`release_watch` 与 FsWatcher 回调
    /// 都会把 map 移入 `tokio::spawn` 的任务，而 `DashMap::clone()` 是
    /// 深拷贝（克隆全部键值对），深拷贝副本上的增删不影响原 map；只有
    /// `Arc` 克隆（引用计数）才让任务与原 map 共享同一份数据。
    watchers: Arc<DashMap<String, Arc<FsWatcher>>>,
    /// workdir → 关注它的容器 id 列表（含各自世代号）
    containers: Arc<DashMap<String, Vec<(String, u64)>>>,
    /// (workdir, container) → 当前世代号（每次 ensure 递增）
    generations: Arc<DashMap<String, u64>>,
    /// VDFS 变更订阅表（可选，由 VDFS provider 构造期注入）。
    ///
    /// 同一份目录树场景只服务 VDFS 机制：文件变化时把容器 id 翻译成 VDFS 路径
    /// 再投递进**会话订阅者共用的那张表**（[`ChangeSubscriptions::notify`]），
    /// 使 `<根>` 页面不必另开一套监听。投递在表内收敛为恰好一次，
    /// 与前端订阅了几条路径无关。
    vdfs_subs: std::sync::Mutex<Option<Arc<ChangeSubscriptions>>>,
}

/// (workdir, container) 的世代守卫键
fn watch_key(workdir: &str, container: &str) -> String {
    format!("{workdir}\u{0}{container}")
}

impl WorkdirWatchManager {
    /// 注入 VDFS 变更订阅表（VDFS provider 构造期调用一次）。
    ///
    /// 未注入时（provider 尚未构造）目录树变化无处可投——VDFS 是工作目录
    /// 变化的**唯一**实时出口，不再有第二条频道；注入后按容器翻译成
    /// [`VdfsChange`] 投进去，`<根>` 页面即可实时刷新。
    pub fn set_vdfs_subs(&self, subs: Arc<ChangeSubscriptions>) {
        if let Ok(mut slot) = self.vdfs_subs.lock() {
            *slot = Some(subs);
        }
    }

    /// 确保该 workdir 的监听已启动（幂等；container 记入关注清单并递增世代）
    pub fn ensure_watch(&self, workdir: &str, container: &str) {
        let key = watch_key(workdir, container);
        let generation = {
            let mut g = self.generations.entry(key).or_insert(0);
            *g += 1;
            *g
        };
        {
            let mut list = self.containers.entry(workdir.to_string()).or_default();
            match list.iter_mut().find(|(c, _)| c == container) {
                Some(entry) => entry.1 = generation,
                None => list.push((container.to_string(), generation)),
            }
        }
        if self.watchers.contains_key(workdir) {
            return;
        }

        let coalesce = Arc::new(std::sync::Mutex::new(CoalesceState::default()));
        let containers = self.containers.clone();
        let vdfs = self.vdfs_subs.lock().ok().and_then(|s| s.clone());
        let wd = workdir.to_string();
        let watcher = Arc::new(FsWatcher::new_with_callback(move |abs_path| {
            let rel = Path::new(&abs_path)
                .strip_prefix(&wd)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();

            // 易变目录过滤：构建产物 / 依赖目录的事件是噪声洪流源，
            // 逐事件丢弃（场景知识：代码工作目录的约定忽略集）
            if rel.split('/').any(|seg| VOLATILE_DIRS.contains(&seg)) {
                return;
            }

            // 事件合并（尾沿去抖）：notify 对一次构建/工具活动会产生成百上千
            // 事件，逐事件发布会打满事件总线长连接、拖垮前端。每个 workdir
            // 至多每 COALESCE_WINDOW 发布一条 VDFS 变更（首个事件即调度，
            // 窗口内的后续事件合并进去）。
            let should_flush = {
                let mut st = coalesce.lock().unwrap();
                if st.scheduled {
                    st.dirty = true;
                    false
                } else {
                    st.scheduled = true;
                    true
                }
            };
            if !should_flush {
                return;
            }

            let st = coalesce.clone();
            let t_containers = containers.clone();
            let t_vdfs = vdfs.clone();
            let t_wd = wd.clone();
            let t_rel = rel.clone();
            tokio::spawn(async move {
                tokio::time::sleep(COALESCE_WINDOW).await;
                let mut g = st.lock().unwrap();
                g.scheduled = false;
                let dirty = g.dirty;
                g.dirty = false;
                drop(g);
                if !dirty {
                    return; // 窗口内无后续变化，且首个事件已由调度方发布
                }
                publish_vdfs_change(&t_vdfs, &t_containers, &t_wd, &t_rel);
            });
            publish_vdfs_change(&vdfs, &containers, &wd, &rel);
        }));

        let w = watcher.clone();
        let root = PathBuf::from(workdir);
        tokio::spawn(async move {
            if let Err(e) = w.start(root).await {
                crate::plugin_error!("session", format!("[Workdir] 监听出错: {e}"));
            }
        });
        self.watchers.insert(workdir.to_string(), watcher);
    }

    /// 释放某容器对 workdir 的订阅（树视图卸载）。
    ///
    /// 世代守卫 + 宽限延迟：仅当宽限期内该 (workdir, container) 没有发生
    /// 重新订阅（世代号不变）才真正移除；最后一个订阅方移除后监听停止
    /// （drop 释放 OS 句柄）。迟到的 unwatch 因世代号已推进而被安全忽略。
    pub fn release_watch(&self, workdir: &str, container: &str) {
        let key = watch_key(workdir, container);
        let Some(generation) = self.generations.get(&key).map(|g| *g) else {
            return; // 从未订阅过
        };

        let containers = self.containers.clone();
        let watchers = self.watchers.clone();
        let generations = self.generations.clone();
        let wd = workdir.to_string();
        let container_owned = container.to_string();
        tokio::spawn(async move {
            tokio::time::sleep(RELEASE_GRACE).await;
            // 世代守卫：期间发生重新订阅 → 本次释放作废
            if generations.get(&key).map(|g| *g) != Some(generation) {
                return;
            }
            // 移除该容器并判断是否仍有订阅方。注意：不能在持有 get_mut 的
            // RefMut（分片写锁）时再对同一分片 remove——DashMap 不可重入，
            // 最后一位订阅方释放时（列表变空恰好走 remove 分支）会永久死锁，
            // 导致监听句柄与条目泄漏。先取结果、释放守卫，再执行移除。
            let empty = containers
                .get_mut(&wd)
                .map(|mut list| {
                    list.retain(|(c, _)| *c != container_owned);
                    list.is_empty()
                })
                .unwrap_or(false);
            if !empty {
                return;
            }
            // 释放守卫后二次确认：窄化「确认空 → 移除」窗口内并发重新订阅
            // 造成新订阅方被误删的可能
            let still_empty = containers
                .get(&wd)
                .map(|list| list.is_empty())
                .unwrap_or(false);
            if still_empty {
                containers.remove(&wd);
                if let Some((_, watcher)) = watchers.remove(&wd) {
                    crate::plugin_info!("session", "[Workdir] 监听已停止: {wd}");
                    drop(watcher);
                }
            }
        });
    }

    /// 测试辅助：该 workdir 是否当前持有监听器
    #[cfg(test)]
    pub fn is_watched(&self, workdir: &str) -> bool {
        self.watchers.contains_key(workdir)
    }
}

/// 释放宽限期：覆盖「卸载的 unwatch 在途 + 返回页面的重新 watch」窗口
const RELEASE_GRACE: std::time::Duration = std::time::Duration::from_secs(2);

/// 事件合并状态（每个 workdir 一份；回调线程与合并任务共享）
#[derive(Default)]
struct CoalesceState {
    /// 是否已有合并任务在途（在途期间事件只置 dirty）
    scheduled: bool,
    /// 合并窗口内又有变化 → 尾沿再发一条
    dirty: bool,
}

/// 合并窗口：至多每秒发布一条 VDFS 变更
const COALESCE_WINDOW: std::time::Duration = std::time::Duration::from_secs(1);

/// 易变目录（构建产物 / 依赖 / 版本库内部）：其变化不作为工作目录数据变更下发
const VOLATILE_DIRS: &[&str] = &[
    "target",
    "node_modules",
    ".git",
    "dist",
    "build",
    ".venv",
    "venv",
    "__pycache__",
    ".next",
    ".cache",
];

/// 把目录树事件翻译为 VDFS 变更并投递进订阅表（每个关注该 workdir 的容器一条）。
///
/// 路径是 VDFS 口径：`<容器 id>/工作目录[/<相对路径>]`——与 VDFS provider
/// 的路径解析严格同一套（见 `SessionPlugin` 的 `parse_session_path`）。
///
/// 为什么要按容器逐条枚举：多个会话可以共享同一个工作目录，订阅表按路径前缀
/// 匹配，只有把每条路径带上各自的容器 id，对应会话的那条订阅才会命中。
/// 未注入订阅表时静默跳过。
fn publish_vdfs_change(
    subs: &Option<Arc<ChangeSubscriptions>>,
    containers: &DashMap<String, Vec<(String, u64)>>,
    workdir: &str,
    rel: &str,
) {
    let Some(subs) = subs else {
        return;
    };
    if let Some(list) = containers.get(workdir) {
        for (container, _) in list.iter() {
            let path = if rel.is_empty() {
                format!("{container}/{SEG_WORKDIR}")
            } else {
                format!("{container}/{SEG_WORKDIR}/{rel}")
            };
            subs.notify(&VdfsChange::bare(path));
        }
    }
}

#[cfg(test)]
#[path = "workdir.test.rs"]
mod tests;
