//! 会话工作目录的 VDFS 场景实现（`.vdfs/session/<id>/工作目录/<rel>`）
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
//! 与绝对路径，并在 join 后做前缀校验（双重闸门，与 agent bundle 的
//! 路径白名单同风格）。

use crate::symbio_core::event_bus::EventBus;
use crate::symbio_core::vdfs::VdfsChange;
use crate::symbio_core::vdfs_provider::{VdfsAccess, VdfsNode};
use crate::symbio_core::{PluginError, PLUGIN_SESSION};
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
/// 文件变化时向所有关注该 workdir 的容器发布粗粒度 `data` 事件（§2.4，
/// kind = 会话、sessionId = 容器 id），驱动前端树视图与详情编辑器按
/// §3.3 防抖重载。机制层只约定「容器数据已变更」语义，不感知文件细节。
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
    /// VDFS 变更广播（可选，由 VDFS provider 构造期注入）。
    ///
    /// 同一份目录树场景只服务 VDFS 机制：文件变化时把容器 id 翻译成 VDFS 路径
    /// 再广播（`VdfsChange`），使 `.vdfs` 页面不必另开一套监听。
    vdfs_tx: std::sync::Mutex<Option<tokio::sync::broadcast::Sender<VdfsChange>>>,
}

/// (workdir, container) 的世代守卫键
fn watch_key(workdir: &str, container: &str) -> String {
    format!("{workdir}\u{0}{container}")
}

impl WorkdirWatchManager {
    /// 注入 VDFS 变更广播源（VDFS provider 构造期调用一次）。
    ///
    /// 未注入广播源时 VDFS 侧不感知（目录树变化仍只发会话频道的粗粒度 `data`
    /// 事件）；注入后同一批事件额外广播为 [`VdfsChange`]，`.vdfs` 页面即可实时刷新。
    pub fn set_vdfs_sender(&self, tx: tokio::sync::broadcast::Sender<VdfsChange>) {
        if let Ok(mut slot) = self.vdfs_tx.lock() {
            *slot = Some(tx);
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
        let vdfs = self.vdfs_tx.lock().ok().and_then(|s| s.clone());
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
            // 至多每 COALESCE_WINDOW 发布一条 `data` 事件（首个事件即调度，
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
                publish_data_event(&t_containers, &t_wd, &t_rel);
                publish_vdfs_change(&t_vdfs, &t_containers, &t_wd, &t_rel);
            });
            publish_data_event(&containers, &wd, &rel);
            publish_vdfs_change(&vdfs, &containers, &wd, &rel);
        }));

        let w = watcher.clone();
        let root = PathBuf::from(workdir);
        tokio::spawn(async move {
            if let Err(e) = w.start(root).await {
                crate::plugin_error!("session", format!("workdir watch error: {e}"));
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
                    crate::plugin_info!("session", "workdir watch stopped: {wd}");
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

/// 合并窗口：至多每秒向总线发布一条 `data` 事件
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

/// 向所有关注该 workdir 的容器发布粗粒度 `data` 事件（§2.4）
fn publish_data_event(containers: &DashMap<String, Vec<(String, u64)>>, workdir: &str, rel: &str) {
    if let Some(list) = containers.get(workdir) {
        for (container, _) in list.iter() {
            EventBus::try_publish(
                PLUGIN_SESSION,
                Some(container),
                json!({ "type": "data", "workdir": workdir, "path": rel }),
            );
        }
    }
}

/// 把目录树事件翻译为 VDFS 变更并广播（每个关注该 workdir 的容器一条）。
///
/// 路径是 VDFS 口径：`<容器 id>/工作目录[/<相对路径>]`——与 VDFS provider
/// 的路径解析严格同一套（见 `SessionPlugin` 的 `parse_session_path`）。
/// 未注入广播源时静默跳过。
fn publish_vdfs_change(
    tx: &Option<tokio::sync::broadcast::Sender<VdfsChange>>,
    containers: &DashMap<String, Vec<(String, u64)>>,
    workdir: &str,
    rel: &str,
) {
    let Some(tx) = tx else {
        return;
    };
    if let Some(list) = containers.get(workdir) {
        for (container, _) in list.iter() {
            let path = if rel.is_empty() {
                format!("{container}/{SEG_WORKDIR}")
            } else {
                format!("{container}/{SEG_WORKDIR}/{rel}")
            };
            let _ = tx.send(VdfsChange::new(
                path,
                crate::symbio_core::vdfs::VDFS_CHANGE_UPDATED,
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    /// 世代守卫延迟释放：卸载的 unwatch 迟到时不得误杀重新建立的订阅；
    /// 宽限期内无重新订阅才真正释放（tokio paused time 驱动）。
    #[tokio::test(start_paused = true)]
    async fn release_watch_generation_guard() {
        let mgr = WorkdirWatchManager::default();
        let wd = "D:/work";

        // 第一次进入：订阅（世代 1）
        mgr.ensure_watch(wd, "sess_a");
        assert!(mgr.is_watched(wd));

        // 切走 → unwatch（世代 1 的释放进入宽限期）
        mgr.release_watch(wd, "sess_a");
        assert!(mgr.is_watched(wd), "宽限期内监听保持");

        // 快速返回 → 重新订阅（世代 2）
        mgr.ensure_watch(wd, "sess_a");
        assert!(mgr.is_watched(wd));

        // 宽限期流逝：世代 1 的迟到释放因世代推进被忽略
        advance_until(&mgr, wd, true).await;
        assert!(mgr.is_watched(wd), "迟到的 unwatch 不得误杀重新订阅");

        // 再切走 → 世代 2 的释放正常生效（无重新订阅推进世代）
        mgr.release_watch(wd, "sess_a");
        advance_until(&mgr, wd, false).await;
        assert!(!mgr.is_watched(wd), "无订阅方后监听应停止");
    }

    /// 在 paused 时钟下推进时间并等待释放任务收敛到目标状态。
    ///
    /// 释放任务经 `tokio::spawn` + `sleep(RELEASE_GRACE)` 调度，定时器在任务
    /// 首次被 poll 时才注册；若首次 poll 发生在时钟推进之后，deadline 会顺延
    /// 到「当前时刻 + 宽限期」。固定 advance 一次不可靠，故循环推进并让出，
    /// 直到目标状态出现（有界，防死循环）。
    async fn advance_until(mgr: &WorkdirWatchManager, wd: &str, watched: bool) {
        for _ in 0..10 {
            tokio::time::advance(RELEASE_GRACE).await;
            tokio::task::yield_now().await;
            if mgr.is_watched(wd) == watched {
                return;
            }
        }
    }

    /// 共享 workdir：多方订阅引用计数，最后一位释放后监听停止
    #[tokio::test(start_paused = true)]
    async fn shared_workdir_reference_counting() {
        let mgr = WorkdirWatchManager::default();
        let wd = "D:/shared";

        mgr.ensure_watch(wd, "sess_a");
        mgr.ensure_watch(wd, "sess_b");
        assert!(mgr.is_watched(wd));

        mgr.release_watch(wd, "sess_a");
        advance_until(&mgr, wd, true).await;
        assert!(mgr.is_watched(wd), "仍有其他订阅方，监听保持");

        mgr.release_watch(wd, "sess_b");
        advance_until(&mgr, wd, false).await;
        assert!(!mgr.is_watched(wd), "最后一位释放后监听停止");
    }

    async fn seed(dir: &Path) {
        tokio::fs::create_dir_all(dir.join("src")).await.unwrap();
        tokio::fs::create_dir_all(dir.join(".hidden"))
            .await
            .unwrap();
        tokio::fs::write(dir.join("src/lib.rs"), "fn main() {}")
            .await
            .unwrap();
        tokio::fs::write(dir.join("README.md"), "# demo")
            .await
            .unwrap();
        tokio::fs::write(dir.join(".env"), "SECRET=1")
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn root_children_skip_hidden_and_sort_dirs_first() {
        let tmp = TempDir::new().unwrap();
        seed(tmp.path()).await;
        let nodes = list_children(tmp.path().to_str().unwrap(), None)
            .await
            .unwrap();
        let names: Vec<&str> = nodes.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, vec!["src", "README.md"]);
        // 是否可展开只看访问位，不看 kind
        assert!(nodes[0].is_dir(), "目录 → `l`");
        assert!(!nodes[1].is_dir(), "文件 → 不可列");
        assert_eq!(nodes[1].access.flags(), "rw");
    }

    #[tokio::test]
    async fn nested_level_reports_only_its_own_segment() {
        let tmp = TempDir::new().unwrap();
        seed(tmp.path()).await;
        let nodes = list_children(tmp.path().to_str().unwrap(), Some("src"))
            .await
            .unwrap();
        assert_eq!(nodes.len(), 1);
        // 全路径由分发层按请求路径回填，本层只给段名
        assert_eq!(nodes[0].name, "lib.rs");
    }

    #[tokio::test]
    async fn traversal_paths_are_rejected() {
        let tmp = TempDir::new().unwrap();
        seed(tmp.path()).await;
        let wd = tmp.path().to_str().unwrap();
        assert!(list_children(wd, Some("../..")).await.is_err());
        assert!(read_node(wd, "../secrets").await.is_err());
    }

    /// `stat` 只回答节点形状，正文归 `vdfs/read`（不再内联，也不误读大文件）
    #[tokio::test]
    async fn stat_shapes_the_node_without_inlining_content() {
        let tmp = TempDir::new().unwrap();
        seed(tmp.path()).await;
        let wd = tmp.path().to_str().unwrap();
        let f = read_node(wd, "README.md").await.unwrap();
        assert_eq!(f.name, "README.md");
        assert_eq!(f.size, Some(6));
        assert_eq!(
            f.attributes.get(ATTR_CONFIG_TYPE).and_then(|v| v.as_str()),
            Some("file")
        );
        let d = read_node(wd, "src").await.unwrap();
        assert!(d.is_dir());
        assert!(d.size.is_none(), "目录没有字节数语义");
        assert_eq!(read_content(wd, "README.md").await.unwrap(), "# demo");
    }

    /// 工作目录**往返**：写 → 列 → 读 → 删（S6 会话内部链路的真实 IO 闭合）
    ///
    /// VDFS 侧的 `<id>/工作目录[/<rel>]` 最终全部落到这四个函数上，而此前
    /// 只有「列 / 读既有文件」的单点测试——写入与删除的实际落盘没有闭合验证。
    #[tokio::test]
    async fn workdir_roundtrip_write_list_read_delete() {
        let tmp = TempDir::new().unwrap();
        seed(tmp.path()).await;
        let wd = tmp.path().to_str().unwrap();

        // 写入（新建子目录内的文件）
        write_node(wd, "src/new.rs", "pub fn new() {}")
            .await
            .unwrap();
        let nested = list_children(wd, Some("src")).await.unwrap();
        let names: Vec<&str> = nested.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, vec!["lib.rs", "new.rs"], "写入后出现在清单中");

        // 读回（VDFS `read` 走 `read_content`）
        assert_eq!(
            read_content(wd, "src/new.rs").await.unwrap(),
            "pub fn new() {}"
        );

        // 覆盖
        write_node(wd, "src/new.rs", "// 改后").await.unwrap();
        assert_eq!(read_content(wd, "src/new.rs").await.unwrap(), "// 改后");
        assert_eq!(
            list_children(wd, Some("src")).await.unwrap().len(),
            2,
            "覆盖不新增"
        );

        // 删除
        delete_node(wd, "src/new.rs").await.unwrap();
        let nested = list_children(wd, Some("src")).await.unwrap();
        assert_eq!(nested.len(), 1);
        assert_eq!(nested[0].name, "lib.rs");

        // 越界写入 / 读取目录内容均被拒（沙箱边界是这条链路的安全底线）
        assert!(write_node(wd, "../escape.rs", "x").await.is_err());
        assert!(read_content(wd, "src").await.is_err(), "目录不可当文件读");
    }

    #[tokio::test]
    async fn workdir_metadata_extracts() {
        let mut s = Session::new("s1");
        assert!(workdir_of(&s).is_none());
        s.metadata["workdir"] = json!("D:/work");
        assert_eq!(workdir_of(&s).as_deref(), Some("D:/work"));
    }
}
