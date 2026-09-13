//! VDFS 访问层（vdfs 插件侧）—— 只做「根 + 全路径」的机械转发
//!
//! ## 本层职责（就这两件）
//!
//! 1. **前端链路**：把 `vdfs/*` 协议请求翻译成对 **VDFS 根 provider** 的调用，
//!    结果装进线路信封（[`super::protocol`]）；
//! 2. **LLM 链路**：同一份翻译经 [`super::tools`] 暴露为工具。
//!
//! ## 本层不认识挂载点（拓扑不在访问层）
//!
//! 虚拟根 `/` 归**组合容器**所有：容器把自己的组合视图注册为 VDFS 根
//! （`CapabilityVisitor::register_vdfs_root`，见 `plugins/composite/vdfs.rs`）。
//! 本层只取这个根，把**规范化后的全路径**原样交给它——
//!
//! ```text
//! 前端 / LLM ──vdfs/*──▶ 本层（翻译）──▶ 根 provider（composite）
//!                                          └─ VdfsMountTable 拆挂载名 / 回填全路径
//! ```
//!
//! 「首段 = 挂载名、相对路径、跨挂载点守卫、事件补全路径」这些拓扑语义全部在
//! 根之后（[`VdfsMountTable`]）；本层因此**不持有任何拓扑知识**：
//! 根之下有多少子树、叫什么名字，本层完全不知道、也不必知道。
//! 于是「新增资源 = 新挂载点」不需要改动本层任何一行。
//!
//! ## 根从哪来
//!
//! - **LLM 链路**：`ctx` 已携带 `CAPABILITY_VISITOR`（编排方 `attach_capabilities`
//!   注入），容器在那次广播里把根注册了进去，直接 `get_vdfs_root()` 即可；
//! - **前端链路**：`ctx` 无能力管理器，从父插件广播一次
//!   `traverse(TRAVERSE_AVAILABLE_TOOLS)`，容器在广播中把根注册进新收集器。
//!
//! 两条链路取到的是**同一个根**。取不到（无组合容器）时降级为**空文件系统**
//! （空 [`VdfsMountTable`]）而非报错——「系统没有资源」与「资源为空」表现一致，
//! 前端与 LLM 都不必特判。

use super::protocol::*;
use crate::symbio_core::vdfs::vdfs_context;
use crate::symbio_core::vdfs_provider::*;
use crate::symbio_core::{
    CapabilityVisitor, DefaultToolVisitor, InvokeRequest, InvokeRequestExt, InvokeResponse, Plugin,
    PluginPayload, CAPABILITY_VISITOR, PATH, TRAVERSE_AVAILABLE_TOOLS, WORKDIR,
};
use std::collections::VecDeque;
use std::sync::Arc;

/// 变更事件在宿主事件总线上的 `kind`
/// （宿主前端：`subscribe({ kind: 'vdfs' })`）
pub const VFDS_EVENT_KIND: &str = "vdfs";

// ==================== 根 provider ====================

/// 空文件系统：无容器注册根时的降级对象。
///
/// `/` 可列出（内容为空），其余路径一律 `NotFound`——语义与「有容器但没有资源」
/// 完全一致，消费者无需为「没有容器」写第二条分支。
pub fn empty_root() -> DynVdfsProvider {
    Arc::new(VdfsMountTable::new(Vec::new()))
}

/// 从能力管理器取容器注册的根；未注册时降级为空文件系统。
///
/// 调用方需已确保 `visitor` 存在（LLM 链路里它是硬前提，缺失应显式报错）。
pub async fn root_of(visitor: &Arc<dyn CapabilityVisitor>) -> DynVdfsProvider {
    visitor.get_vdfs_root().await.unwrap_or_else(empty_root)
}

/// 取本次调用的 VDFS 根（前端链路入口）。
///
/// `ctx` 已带能力管理器时直接复用（不重复广播）；否则从 `parent` 广播一次，
/// 让容器把自己的组合视图注册进新的收集器。
pub async fn resolve_root(
    parent: Option<&Arc<dyn Plugin>>,
    ctx: &Arc<dyn InvokeRequest>,
) -> DynVdfsProvider {
    if let Some(visitor) = ctx.get(CAPABILITY_VISITOR) {
        return root_of(&visitor).await;
    }

    let manager: Arc<dyn CapabilityVisitor> = Arc::new(DefaultToolVisitor::new());
    if let Some(parent) = parent {
        let sub = ctx.fork();
        sub.set(PATH, TRAVERSE_AVAILABLE_TOOLS.to_string());
        sub.set(CAPABILITY_VISITOR, manager.clone());
        if let Err(e) = parent.clone().traverse(String::new(), sub).await {
            crate::plugin_warn!("vdfs", "resolve_root: 广播失败，降级为空文件系统: {e:?}");
        }
    }
    root_of(&manager).await
}

// ==================== 变更投递 ====================

/// 从全路径取首段作为挂载名（虚拟根 → 空串）
fn mount_of(full: &str) -> String {
    full.trim_start_matches('/')
        .split('/')
        .next()
        .unwrap_or("")
        .to_string()
}

/// provider 报出的路径 → 总线事件。
///
/// provider 只报**子树内相对路径**，根（[`VdfsMountTable`]）在转发时已把它拼成
/// 全路径，因此本层只需从全路径里取回顾首段作为挂载名，不再做任何拼装。
fn to_change_event(change: &VdfsChange) -> VdfsChangeEvent {
    let abs = |p: &str| {
        if p.starts_with('/') {
            p.to_string()
        } else {
            format!("/{p}")
        }
    };
    VdfsChangeEvent {
        mount: mount_of(&change.path),
        path: abs(&change.path),
        change: change.change.clone(),
        to: change.to.as_deref().map(abs),
    }
}

/// 构造变更投递器：接到全局事件总线，下发前端（`kind = "vdfs"`）。
fn event_bus_sink() -> VdfsChangeSink {
    Arc::new(move |change: VdfsChange| {
        let event = to_change_event(&change);
        let data = serde_json::to_value(&event).unwrap_or(serde_json::Value::Null);
        crate::symbio_core::event_bus::EventBus::try_publish(VFDS_EVENT_KIND, None, data);
    })
}

// ==================== 请求载荷 ====================

/// 读取请求载荷（缺省容忍空载荷）
fn payload_or_default<T: serde::de::DeserializeOwned + Default + Clone + Send + Sync + 'static>(
    ctx: &Arc<dyn InvokeRequest>,
) -> T {
    ctx.payload::<serde_json::Value>()
        .ok()
        .and_then(|v| serde_json::from_value::<T>(v).ok())
        .unwrap_or_default()
}

// ==================== 调用级参数 ====================

/// 宿主 ctx → VDFS 调用级参数：把运行时状态翻译成 provider 的**约定键**。
///
/// 这里是唯一的翻译点——provider 只认 [`VFDS_PARAM_WORKDIR`] 这类约定键，
/// 不认识宿主 ctx 的键名（`WORKDIR` 等）。两条链路共用：
/// 前端协议入口（`/[挂载点]/…` 全路径）与 LLM 工具（`local/…` 本地地址）。
pub fn call_params(ctx: &Arc<dyn InvokeRequest>) -> VdfsParams {
    let mut params = VdfsParams::new();
    if let Some(workdir) = ctx.get(WORKDIR) {
        params.insert(
            VFDS_PARAM_WORKDIR.to_string(),
            serde_json::Value::String(workdir),
        );
    }
    params
}

// ==================== 路径回填（兜底） ====================

/// 子节点全路径（`base` = 父目录全路径；`base == "/"` → `/name`）
fn child_path(base: &str, name: &str) -> String {
    let base = base.trim_end_matches('/');
    if base.is_empty() {
        format!("/{name}")
    } else {
        format!("{base}/{name}")
    }
}

/// 兜底回填：根 provider 未填 `path` / `ext` / `title` 时按请求路径补齐。
///
/// 根（[`VdfsMountTable`]）本身已回填，这里是**协议通用兜底**——任何 provider
/// 实现都可以只填 `name` 与 `access`，其余由访问层补全。
fn fill_paths(base: &str, nodes: &mut [VdfsNode]) {
    for n in nodes.iter_mut() {
        if n.path.is_empty() {
            n.path = child_path(base, &n.name);
        }
        if n.ext.is_none() {
            n.ext = derive_ext(&n.name);
        }
        if n.title.is_empty() {
            n.title = n.name.clone();
        }
    }
}

/// 目录自身节点的兜底（provider 未实现 `stat` 时）
fn dir_self(full: &str) -> VdfsNode {
    let name = if full == VFDS_ROOT {
        VFDS_ROOT.to_string()
    } else {
        full.rsplit('/').next().unwrap_or(full).to_string()
    };
    let mut n = VdfsNode::dir(name.clone(), name, VdfsAccess::LIST);
    n.path = full.to_string();
    n
}

// ==================== 统一分发 ====================

/// `vdfs/*` 统一分发入口。
///
/// 返回 `None` 表示该 path 不是 VDFS 协议路径（调用方继续自己的 match）：
///
/// ```ignore
/// if let Some(resp) = host::dispatch_with(&root, path, &ctx, params).await {
///     return resp;
/// }
/// ```
///
/// `params` 是**调用级参数**：调用方把请求 ctx 里的运行时状态（如 workdir）按约定键
/// 透传给 provider，provider 因此不必知道宿主 ctx 的键名约定（见 [`VFDS_PARAM_WORKDIR`]）。
/// 不需要任何参数时传空的 [`VdfsParams`]。
pub async fn dispatch_with(
    root: &DynVdfsProvider,
    path: &str,
    ctx: &Arc<dyn InvokeRequest>,
    params: VdfsParams,
) -> Option<InvokeResponse<PluginPayload>> {
    if !VFDS_OPS.contains(&path) {
        return None;
    }
    let vctx = vdfs_context(ctx).with_params(params);
    let resp = match path {
        VFDS_PROVIDERS => providers(root, &vctx).await,
        VFDS_LIST => list(root, &vctx, ctx).await,
        VFDS_TREE => tree(root, &vctx, ctx).await,
        VFDS_STAT => stat(root, &vctx, ctx).await,
        VFDS_READ => read(root, &vctx, ctx).await,
        VFDS_WRITE => write(root, &vctx, ctx).await,
        VFDS_DELETE => delete(root, &vctx, ctx).await,
        VFDS_MKDIR => mkdir(root, &vctx, ctx).await,
        VFDS_MOVE => move_item(root, &vctx, ctx).await,
        VFDS_EDIT => edit(root, &vctx, ctx).await,
        VFDS_SEARCH => search(root, &vctx, ctx).await,
        VFDS_WATCH | VFDS_UNWATCH => watch(root, &vctx, ctx, path == VFDS_WATCH).await,
        _ => unreachable!("VFDS_OPS 与分发分支必须一一对应"),
    };
    Some(resp)
}

/// `vdfs/providers` —— 虚拟根 `/` 的目录内容，装成便于导航的使用方视图。
///
/// 根的 `list("/")` 就是挂载点清单（组合视图保证），因此本操作只是 `list("/")`
/// 的另一种呈现，**不需要任何额外拓扑知识**：`label` / `root` 都有缺省。
async fn providers(root: &DynVdfsProvider, vctx: &VdfsContext) -> InvokeResponse<PluginPayload> {
    let mut nodes = root.list(vctx, VFDS_ROOT).await?;
    fill_paths(VFDS_ROOT, &mut nodes);
    let infos: Vec<VdfsMountInfo> = nodes
        .into_iter()
        .enumerate()
        .map(|(i, n)| mount_info(i as i32, n))
        .collect();
    Ok(PluginPayload::new(&VdfsProvidersResponse {
        providers: infos,
    }))
}

/// 挂载点节点 → 使用方视图（不依赖任何 provider 专有字段）
fn mount_info(order: i32, n: VdfsNode) -> VdfsMountInfo {
    let mount = n.name.clone();
    VdfsMountInfo {
        mount: mount.clone(),
        label: if n.title.is_empty() {
            mount.clone()
        } else {
            n.title.clone()
        },
        description: n.description.clone(),
        order,
        access: n.access,
        status: if n.status.is_empty() {
            VFDS_STATUS_ACTIVE.to_string()
        } else {
            n.status.clone()
        },
        root: if n.path.is_empty() {
            format!("/{mount}")
        } else {
            n.path.clone()
        },
        icon: None,
        new_types: n.new_types.clone(),
        // 导航可见性：来自 provider 声明，经挂载点节点的场景属性透传
        // （缺省 true，节点未标注即视为可见）
        nav_visible: n
            .attributes
            .get(VFDS_ATTR_NAV_VISIBLE)
            .and_then(|v| v.as_bool())
            .unwrap_or(true),
        attributes: n.attributes.clone(),
    }
}

async fn list(
    root: &DynVdfsProvider,
    vctx: &VdfsContext,
    ctx: &Arc<dyn InvokeRequest>,
) -> InvokeResponse<PluginPayload> {
    let req: VdfsListRequest = payload_or_default(ctx);
    let full = normalize_path(&req.path)?;

    let mut items = root.list(vctx, &full).await?;
    fill_paths(&full, &mut items);

    // 目录自身节点：provider 未实现 stat 时按目录形态兜底
    let node = match root.stat(vctx, &full).await {
        Ok(mut n) => {
            if n.title.is_empty() {
                n.title = n.name.clone();
            }
            if n.ext.is_none() {
                n.ext = derive_ext(&n.name);
            }
            if n.path.is_empty() {
                n.path = full.clone();
            }
            n
        }
        Err(_) => dir_self(&full),
    };

    Ok(PluginPayload::new(&VdfsListResponse {
        path: full,
        node,
        items,
    }))
}

async fn stat(
    root: &DynVdfsProvider,
    vctx: &VdfsContext,
    ctx: &Arc<dyn InvokeRequest>,
) -> InvokeResponse<PluginPayload> {
    let req: VdfsPathRequest = payload_or_default(ctx);
    let full = normalize_path(&req.path)?;
    let mut n = root.stat(vctx, &full).await?;
    if n.title.is_empty() {
        n.title = n.name.clone();
    }
    if n.ext.is_none() {
        n.ext = derive_ext(&n.name);
    }
    if n.path.is_empty() {
        n.path = full;
    }
    Ok(PluginPayload::new(&n))
}

async fn read(
    root: &DynVdfsProvider,
    vctx: &VdfsContext,
    ctx: &Arc<dyn InvokeRequest>,
) -> InvokeResponse<PluginPayload> {
    let req: VdfsReadRequest = payload_or_default(ctx);
    let full = normalize_path(&req.path)?;
    let mut c = root.read(vctx, &full).await?;
    if c.path.is_empty() {
        c.path = full;
    }
    Ok(PluginPayload::new(&c))
}

async fn write(
    root: &DynVdfsProvider,
    vctx: &VdfsContext,
    ctx: &Arc<dyn InvokeRequest>,
) -> InvokeResponse<PluginPayload> {
    let req: VdfsWriteRequest = payload_or_default(ctx);
    let full = normalize_path(&req.path)?;
    // 机制级守卫：写入必须携带内容（语义级校验归 provider）
    if req.text.is_none() && req.b64.is_none() {
        return Err(VdfsError::invalid("写入需要 text 或 b64 之一作为内容").into());
    }
    let content = req.to_content();
    let mut r = root.write(vctx, &full, &content).await?;
    if r.path.is_empty() {
        r.path = full;
    }
    Ok(PluginPayload::new(&r))
}

async fn delete(
    root: &DynVdfsProvider,
    vctx: &VdfsContext,
    ctx: &Arc<dyn InvokeRequest>,
) -> InvokeResponse<PluginPayload> {
    let req: VdfsPathRequest = payload_or_default(ctx);
    let full = normalize_path(&req.path)?;
    root.delete(vctx, &full, req.recursive).await?;
    Ok(PluginPayload::new(&VdfsDeleteResponse { path: full }))
}

async fn mkdir(
    root: &DynVdfsProvider,
    vctx: &VdfsContext,
    ctx: &Arc<dyn InvokeRequest>,
) -> InvokeResponse<PluginPayload> {
    let req: VdfsPathRequest = payload_or_default(ctx);
    let full = normalize_path(&req.path)?;
    root.mkdir(vctx, &full).await?;
    Ok(PluginPayload::new(&VdfsWriteResponse {
        path: full,
        created: true,
        etag: None,
    }))
}

async fn move_item(
    root: &DynVdfsProvider,
    vctx: &VdfsContext,
    ctx: &Arc<dyn InvokeRequest>,
) -> InvokeResponse<PluginPayload> {
    let req: VdfsMoveRequest = payload_or_default(ctx);
    let from = normalize_path(&req.from)?;
    let to = normalize_path(&req.to)?;
    // 跨挂载点拒绝由根（VdfsMountTable）判定——本层只传全路径
    root.move_item(vctx, &from, &to).await?;
    Ok(PluginPayload::new(&VdfsMoveResponse { from, to }))
}

// ==================== 组合操作（edit / search）====================
//
// provider 只出**原子操作**（list / stat / read / write / delete / mkdir / move）；
// 组合逻辑在访问层**只写一次**——前端协议入口用下面的 handler，LLM 工具链路
// （`provider::ToolVdfs`）直接调用 `edit_via` / `search_via`，任何 `VdfsProvider`
// 实现方都无需重复实现这些逻辑。

/// 统一换行符为 `\n`（用于精确替换匹配，与原生 `file_edit` 一致）
fn normalize_line_endings(s: &str) -> String {
    s.replace("\r\n", "\n").replace('\r', "\n")
}

/// 探测原文换行符风格（CRLF 优先），替换后恢复原风格
fn find_line_ending_style(content: &str) -> &'static str {
    if content.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

/// 编辑 = `read` → 精确替换 → `write` 的组合。
///
/// `provider` + `rel` 由调用方给定（前端链路 = 根 + 全路径；工具链路 = 挂载
/// provider + 子树相对路径）；安全规则（白名单 / 拒符号链接写）经 provider 的
/// `read` / `write` 自持生效，本函数不重复实现。
pub(crate) async fn edit_via(
    provider: &DynVdfsProvider,
    vctx: &VdfsContext,
    rel: &str,
    old_string: &str,
    new_string: &str,
) -> VdfsResult<VdfsEditResponse> {
    if rel.contains('\0') {
        return Err(VdfsError::invalid("无效路径：不允许 null 字节"));
    }
    if old_string.is_empty() {
        return Err(VdfsError::invalid("old_string 不能为空"));
    }

    let raw = provider.read(vctx, rel).await?;
    let Some(raw_content) = raw.text else {
        return Err(VdfsError::invalid("仅支持编辑文本内容（二进制不可编辑）"));
    };

    // 探明原文换行符风格，规范化后做精确匹配，写回时恢复
    let line_ending = find_line_ending_style(&raw_content);
    let content = normalize_line_endings(&raw_content);
    let old_normalized = normalize_line_endings(old_string);
    let new_normalized = normalize_line_endings(new_string);

    if old_normalized == new_normalized {
        return Ok(VdfsEditResponse {
            path: rel.to_string(),
            replaced: 0,
            message: Some(format!("已检查 {rel}：内容已为最新，无需修改")),
        });
    }

    let match_count = content.matches(&old_normalized).count();
    if match_count == 0 {
        // 安全截断：取前 200 字节内的最后合法字符边界作为上下文
        let context_end = raw_content
            .char_indices()
            .map(|(idx, _)| idx)
            .rfind(|&idx| idx <= 200)
            .unwrap_or(0);
        let context = &raw_content[..context_end];
        return Err(VdfsError::invalid(format!(
            "未找到 old_string 在文件 '{rel}' 中。文件前 {context_end} 字节上下文: {context:?}"
        )));
    }
    if match_count > 1 {
        return Err(VdfsError::invalid(format!(
            "old_string 在文件 '{rel}' 中匹配 {match_count} 次；必须精确匹配一次"
        )));
    }

    let new_content = content.replacen(&old_normalized, &new_normalized, 1);
    let final_content = if line_ending == "\r\n" {
        new_content.replace('\n', "\r\n")
    } else {
        new_content
    };
    let size = final_content.len();

    provider
        .write(vctx, rel, &VdfsContent::text("", final_content))
        .await?;

    Ok(VdfsEditResponse {
        path: rel.to_string(),
        replaced: 1,
        message: Some(format!("已编辑 {rel}：替换了 1 处（{size} 字节）")),
    })
}

/// 单次搜索返回上限（与原生 `glob_search` 一致）
pub(crate) const MAX_SEARCH_RESULTS: usize = 1000;

/// 文件名 Glob 搜索 = 递归 `list` + 模式过滤的组合。
///
/// 沿 [`VdfsProvider::list`] 下钻（`t` 位控制、单分支失败跳过），对每个普通文件
/// 的地址（剥掉前导 `/` 后）做 Glob 匹配；provider 的安全规则（白名单、访问位）
/// 经 `list` 自持生效。`base` 为搜索基目录（空 = 子树根；前端链路给 `/挂载名/…`
/// 全路径，工具链路给子树相对路径），**结果与 `base` 同坐标系**。
pub(crate) async fn search_via(
    provider: &DynVdfsProvider,
    vctx: &VdfsContext,
    base: &str,
    pattern: &str,
) -> VdfsResult<VdfsSearchResult> {
    if pattern.starts_with('/') || pattern.starts_with('\\') {
        return Err(VdfsError::invalid(
            "不允许使用绝对路径。请使用相对 Glob 模式。",
        ));
    }
    if pattern.contains("../") || pattern.contains("..\\") || pattern == ".." {
        return Err(VdfsError::invalid(
            "不允许在 Glob 模式中使用路径遍历 ('..')。",
        ));
    }
    let pat = glob::Pattern::new(pattern)
        .map_err(|e| VdfsError::invalid(format!("无效的 Glob 模式：{e}")))?;

    let mut results: Vec<String> = Vec::new();
    let mut truncated = false;
    let mut queue: VecDeque<String> = VecDeque::new();
    queue.push_back(base.to_string());

    while let Some(dir) = queue.pop_front() {
        let children = match provider.list(vctx, &dir).await {
            Ok(c) => c,
            // 单分支失败降级：不让一棵子树拖垮整个搜索
            Err(e) => {
                crate::plugin_warn!("vdfs", "vdfs_search: 列出 {dir} 失败，已跳过: {e}");
                continue;
            }
        };
        // 子地址 = 父目录 + 子名（`""` 基目录直接用子名，保持相对形态）
        let parent = dir.trim_end_matches('/').to_string();
        for child in children {
            let child_addr = if parent.is_empty() {
                child.name.clone()
            } else {
                format!("{parent}/{}", child.name)
            };
            if !child.is_dir() && pat.matches(child_addr.trim_start_matches('/')) {
                results.push(child_addr.clone());
                if results.len() >= MAX_SEARCH_RESULTS {
                    truncated = true;
                    break;
                }
            }
            if child.access.traverse {
                queue.push_back(child_addr);
            }
        }
        if truncated {
            break;
        }
    }
    results.sort();
    Ok(VdfsSearchResult { results, truncated })
}

async fn edit(
    root: &DynVdfsProvider,
    vctx: &VdfsContext,
    ctx: &Arc<dyn InvokeRequest>,
) -> InvokeResponse<PluginPayload> {
    let req: VdfsEditRequest = payload_or_default(ctx);
    let full = normalize_path(&req.path)?;
    let r = edit_via(root, vctx, &full, &req.old_string, &req.new_string).await?;
    Ok(PluginPayload::new(&r))
}

async fn search(
    root: &DynVdfsProvider,
    vctx: &VdfsContext,
    ctx: &Arc<dyn InvokeRequest>,
) -> InvokeResponse<PluginPayload> {
    let req: VdfsSearchRequest = payload_or_default(ctx);
    let full = normalize_path(&req.path)?;
    let r = search_via(root, vctx, &full, &req.pattern).await?;
    Ok(PluginPayload::new(&r))
}

async fn watch(
    root: &DynVdfsProvider,
    vctx: &VdfsContext,
    ctx: &Arc<dyn InvokeRequest>,
    subscribe: bool,
) -> InvokeResponse<PluginPayload> {
    let req: VdfsPathRequest = payload_or_default(ctx);
    let full = normalize_path(&req.path)?;
    if subscribe {
        root.watch(vctx, &full, event_bus_sink()).await?;
    } else {
        root.unwatch(vctx, &full).await?;
    }
    Ok(PluginPayload::new(
        &crate::symbio_core::schemas::common::SuccessResponse::default(),
    ))
}

/// 树状遍历：访问层统一实现（递归 [`VdfsProvider::list`]）。
///
/// provider 只需实现 `list`，并在目录节点的 `access` 上声明 `t` 位即可被遍历；
/// 机制不含任何场景语义。这里**不需要挂载名**——全路径本身就是递归的地址。
/// 深度与数量上限防爆炸，超限时 `truncated = true`。
async fn tree(
    root: &DynVdfsProvider,
    vctx: &VdfsContext,
    ctx: &Arc<dyn InvokeRequest>,
) -> InvokeResponse<PluginPayload> {
    let req: VdfsTreeRequest = payload_or_default(ctx);
    let full = normalize_path(&req.path)?;
    let depth_limit = req.depth.unwrap_or(3); // 0 = 不限
    let count_limit = req.limit.unwrap_or(500).max(1) as usize;

    let mut out: Vec<VdfsNode> = Vec::new();
    let mut truncated = false;

    // 队列元素 = (目录全路径, 深度)
    let mut queue: VecDeque<(String, u32)> = VecDeque::new();
    queue.push_back((full.clone(), 0));

    while let Some((dir, depth)) = queue.pop_front() {
        let mut children = match root.list(vctx, &dir).await {
            Ok(c) => c,
            // 单分支失败降级：不让一棵子树拖垮整个遍历
            Err(e) => {
                crate::plugin_warn!("vdfs", "tree: 列出 {dir} 失败，已跳过: {e}");
                continue;
            }
        };
        fill_paths(&dir, &mut children);

        for child in children {
            if out.len() >= count_limit {
                truncated = true;
                break;
            }
            let descend = child.access.traverse && (depth_limit == 0 || depth + 1 < depth_limit);
            let path = child.path.clone();
            out.push(child);
            if descend {
                queue.push_back((path, depth + 1));
            }
        }
        if truncated {
            break;
        }
    }

    Ok(PluginPayload::new(&VdfsTreeResponse {
        path: full,
        nodes: out,
        truncated,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbio_core::vdfs_provider::VdfsMountTable;
    use crate::symbio_core::{PluginError, PluginMeta, SimpleRequest};
    use async_trait::async_trait;
    use serde_json::{json, Value};
    use std::sync::Mutex;

    fn ctx_with(payload: Value) -> Arc<dyn InvokeRequest> {
        let ctx = Arc::new(SimpleRequest::new(None, None));
        ctx.set_payload(payload).unwrap();
        ctx
    }

    fn ctx_empty() -> Arc<dyn InvokeRequest> {
        Arc::new(SimpleRequest::new(None, None))
    }

    /// 测试便捷：不带调用级参数的 [`dispatch_with`]（被测链路都不依赖 params）
    async fn dispatch(
        root: &DynVdfsProvider,
        path: &str,
        ctx: &Arc<dyn InvokeRequest>,
    ) -> Option<InvokeResponse<PluginPayload>> {
        dispatch_with(root, path, ctx, VdfsParams::new()).await
    }

    /// 内存子树（**自身不含挂载名**）：记录收到的相对路径，便于断言转发语义。
    ///
    /// `""`    → a.txt (file, rw) / sub (dir, lt)
    /// `"sub"` → b.md (file, r)
    struct Rec {
        seen: Mutex<Vec<String>>,
    }

    impl Rec {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                seen: Mutex::new(Vec::new()),
            })
        }

        fn seen(&self) -> Vec<String> {
            self.seen.lock().unwrap().clone()
        }

        fn note(&self, p: &str) {
            self.seen.lock().unwrap().push(p.to_string());
        }
    }

    #[async_trait]
    impl VdfsProvider for Rec {
        fn label(&self) -> Option<&str> {
            Some("内存子树")
        }
        fn order(&self) -> i32 {
            10
        }
        fn root_access(&self) -> VdfsAccess {
            VdfsAccess::LIST_TRAVERSE
        }

        async fn list(&self, _ctx: &VdfsContext, path: &str) -> VdfsResult<Vec<VdfsNode>> {
            self.note(path);
            Ok(match path {
                "" => vec![
                    VdfsNode::file("a.txt", "A", VdfsAccess::READ_WRITE),
                    VdfsNode::dir("sub", "子目录", VdfsAccess::LIST_TRAVERSE),
                ],
                "sub" => vec![VdfsNode::file("b.md", "B", VdfsAccess::READ)],
                _ => return Err(VdfsError::not_found(format!("无此目录：{path}"))),
            })
        }

        async fn stat(&self, _ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsNode> {
            match path {
                "a.txt" => Ok(VdfsNode::file("a.txt", "A", VdfsAccess::READ_WRITE)),
                "sub" => Ok(VdfsNode::dir("sub", "子目录", VdfsAccess::LIST_TRAVERSE)),
                "sub/b.md" => Ok(VdfsNode::file("b.md", "B", VdfsAccess::READ)),
                _ => Err(VdfsError::not_found(path)),
            }
        }

        async fn read(&self, _ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsContent> {
            self.note(path);
            match path {
                "a.txt" => Ok(VdfsContent::text("", "hello")),
                "sub/b.md" => Ok(VdfsContent::text("", "# b")),
                _ => Err(VdfsError::Forbidden("目录不可读".into())),
            }
        }

        async fn write(
            &self,
            _ctx: &VdfsContext,
            path: &str,
            content: &VdfsContent,
        ) -> VdfsResult<VdfsWriteResponse> {
            // 演示「provider 自持校验 + 字段级错误」
            if content.text.as_deref() == Some("bad") {
                return Err(VdfsError::Invalid(
                    VdfsValidationError::new("内容不合法").with_field("text", "不允许 bad"),
                ));
            }
            Ok(VdfsWriteResponse {
                path: String::new(),
                created: path == "new.txt",
                etag: Some("v1".into()),
            })
        }

        async fn delete(&self, _ctx: &VdfsContext, path: &str, _r: bool) -> VdfsResult<()> {
            if path == "sub" || path == "a.txt" {
                Ok(())
            } else {
                Err(VdfsError::Forbidden(format!("不允许删除 {path}")))
            }
        }

        async fn move_item(&self, _ctx: &VdfsContext, from: &str, to: &str) -> VdfsResult<()> {
            // 只接受相对路径：根（VdfsMountTable）已剥掉挂载前缀
            if from == "a.txt" && to == "b.txt" {
                Ok(())
            } else {
                Err(VdfsError::Forbidden(format!("不支持移动 {from} → {to}")))
            }
        }

        async fn watch(
            &self,
            _ctx: &VdfsContext,
            path: &str,
            _sink: VdfsChangeSink,
        ) -> VdfsResult<()> {
            self.note(path);
            Ok(())
        }

        async fn unwatch(&self, _ctx: &VdfsContext, path: &str) -> VdfsResult<()> {
            self.note(path);
            Ok(())
        }
    }

    /// 极简 provider：验证「什么都不实现」也能被访问层容错
    struct Bare;

    #[async_trait]
    impl VdfsProvider for Bare {}

    /// 以 `rec` 为唯一挂载点的根（模拟 composite 的组合视图）
    fn root_with(rec: &Arc<Rec>) -> DynVdfsProvider {
        let p: DynVdfsProvider = rec.clone();
        Arc::new(VdfsMountTable::new(vec![("mem".to_string(), p)]))
    }

    fn roots() -> (DynVdfsProvider, Arc<Rec>) {
        let rec = Rec::new();
        let root = root_with(&rec);
        (root, rec)
    }

    #[tokio::test]
    async fn dispatch_ignores_non_vdfs_path() {
        let (root, _) = roots();
        let ctx = ctx_empty();
        assert!(dispatch(&root, "chat/send", &ctx).await.is_none());
        assert!(dispatch(&root, "", &ctx).await.is_none());
    }

    /// `vdfs/providers` = 根的 `list("/")`，无需触达任何 provider
    #[tokio::test]
    async fn providers_derive_from_root_listing() {
        let (root, rec) = roots();
        let resp = dispatch(&root, VFDS_PROVIDERS, &ctx_empty())
            .await
            .unwrap()
            .unwrap();
        let data = resp.get::<VdfsProvidersResponse>().unwrap();
        assert_eq!(data.providers.len(), 1);
        let m = &data.providers[0];
        assert_eq!(m.mount, "mem", "挂载名来自注册名");
        assert_eq!(m.root, "/mem");
        assert_eq!(m.label, "内存子树", "label 取自节点 title");
        assert_eq!(m.access.flags(), "lt");
        assert_eq!(
            m.order, 0,
            "order = 在根下的位置（根已按 provider order 排序）"
        );
        assert!(rec.seen().is_empty(), "list(/) 由组合视图内部完成");
    }

    /// 导航可见性经「provider 声明 → 挂载节点属性 → 挂载视图」整链透传
    #[tokio::test]
    async fn providers_carry_nav_visibility() {
        // 隐藏型 provider：除可见性外与 Rec 同构（能力不变）
        struct Hidden;
        #[async_trait]
        impl VdfsProvider for Hidden {
            fn label(&self) -> Option<&str> {
                Some("本地文件")
            }
            fn nav_visible(&self) -> bool {
                false
            }
        }

        let hidden: DynVdfsProvider = Arc::new(Hidden);
        let shown: DynVdfsProvider = Rec::new();
        let root: DynVdfsProvider = Arc::new(VdfsMountTable::new(vec![
            ("local".to_string(), hidden),
            ("mem".to_string(), shown),
        ]));

        let resp = dispatch(&root, VFDS_PROVIDERS, &ctx_empty())
            .await
            .unwrap()
            .unwrap();
        let data = resp.get::<VdfsProvidersResponse>().unwrap();
        assert_eq!(data.providers.len(), 2, "隐藏的子树仍是挂载点");
        let local = data.providers.iter().find(|m| m.mount == "local").unwrap();
        let mem = data.providers.iter().find(|m| m.mount == "mem").unwrap();
        assert!(
            !local.nav_visible,
            "声明不可见的挂载点下传 nav_visible=false"
        );
        assert!(mem.nav_visible, "未声明者缺省可见");
    }

    /// 挂载根列表：全路径回填 + `ext` 推导
    #[tokio::test]
    async fn list_mount_root_fills_paths_and_ext() {
        let (root, rec) = roots();
        let ctx = ctx_with(json!({ "path": "/mem" }));
        let resp = dispatch(&root, VFDS_LIST, &ctx).await.unwrap().unwrap();
        let data = resp.get::<VdfsListResponse>().unwrap();
        assert_eq!(rec.seen(), vec![""], "provider 收到的是相对路径 \"\"");
        assert_eq!(data.path, "/mem");
        assert_eq!(data.node.name, "mem", "目录自身节点 = 挂载根");
        assert_eq!(data.items.len(), 2);
        assert_eq!(data.items[0].path, "/mem/a.txt");
        assert_eq!(data.items[0].ext.as_deref(), Some("txt"));
        assert_eq!(data.items[1].path, "/mem/sub");
        assert!(data.items[1].is_dir());
        assert!(data.items[1].ext.is_none());
    }

    /// 访问层不重写路径：全路径原样穿过，根负责拆分
    #[tokio::test]
    async fn full_paths_pass_through_unchanged() {
        let (root, rec) = roots();
        let ctx = ctx_with(json!({ "path": "/mem/sub" }));
        let resp = dispatch(&root, VFDS_LIST, &ctx).await.unwrap().unwrap();
        let data = resp.get::<VdfsListResponse>().unwrap();
        assert_eq!(rec.seen(), vec!["sub"], "根把 /mem/sub 拆成相对路径 sub");
        assert_eq!(data.items[0].name, "b.md");
        assert_eq!(data.items[0].path, "/mem/sub/b.md");
        assert_eq!(data.node.name, "sub");
    }

    #[tokio::test]
    async fn list_unknown_mount_is_not_found_with_hint() {
        let (root, _) = roots();
        let ctx = ctx_with(json!({ "path": "/nope" }));
        let err = dispatch(&root, VFDS_LIST, &ctx).await.unwrap().unwrap_err();
        assert!(matches!(err, PluginError::NotFound(_)));
        assert!(err.to_string().contains("/mem"), "提示现有挂载点");
    }

    /// 路径穿越在访问层被拦截，根与 provider 永远拿到安全路径
    #[tokio::test]
    async fn traversal_path_rejected() {
        let (root, _) = roots();
        let ctx = ctx_with(json!({ "path": "/mem/../../etc" }));
        let err = dispatch(&root, VFDS_LIST, &ctx).await.unwrap().unwrap_err();
        assert!(matches!(err, PluginError::ValidationError(_)));
    }

    #[tokio::test]
    async fn stat_read_and_backfill() {
        let (root, _) = roots();

        let ctx = ctx_with(json!({ "path": "/mem/a.txt" }));
        let resp = dispatch(&root, VFDS_READ, &ctx).await.unwrap().unwrap();
        let c = resp.get::<VdfsContent>().unwrap();
        assert_eq!(c.text.as_deref(), Some("hello"));
        assert_eq!(c.path, "/mem/a.txt", "provider 未填 path，由访问层回填");

        let ctx = ctx_with(json!({ "path": "/mem/sub/b.md" }));
        let n = dispatch(&root, VFDS_STAT, &ctx)
            .await
            .unwrap()
            .unwrap()
            .get::<VdfsNode>()
            .unwrap();
        assert_eq!(n.path, "/mem/sub/b.md");
        assert_eq!(n.effective_ext().as_deref(), Some("md"));
        assert_eq!(n.access.flags(), "r");

        // 挂载根节点由组合视图合成（provider 不知道自己的挂载名）
        let ctx = ctx_with(json!({ "path": "/mem" }));
        let n = dispatch(&root, VFDS_STAT, &ctx)
            .await
            .unwrap()
            .unwrap()
            .get::<VdfsNode>()
            .unwrap();
        assert_eq!(n.kind, VFDS_KIND_MOUNT);
        assert_eq!(n.name, "mem");
        assert_eq!(n.title, "内存子树");
    }

    /// 机器级守卫：内容缺失即拒绝，provider 不会被调用
    #[tokio::test]
    async fn write_requires_content_and_maps_validation_fields() {
        let (root, _) = roots();

        let ctx = ctx_with(json!({ "path": "/mem/a.txt" }));
        assert!(matches!(
            dispatch(&root, VFDS_WRITE, &ctx).await.unwrap(),
            Err(PluginError::ValidationError(_))
        ));

        let ctx = ctx_with(json!({ "path": "/mem/a.txt", "text": "x" }));
        let w = dispatch(&root, VFDS_WRITE, &ctx)
            .await
            .unwrap()
            .unwrap()
            .get::<VdfsWriteResponse>()
            .unwrap();
        assert_eq!(w.path, "/mem/a.txt");
        assert_eq!(w.etag.as_deref(), Some("v1"));

        // 字段级校验错误：载荷序列化为 JSON 置于错误文案位，可解析还原
        let ctx = ctx_with(json!({ "path": "/mem/a.txt", "text": "bad" }));
        let err = dispatch(&root, VFDS_WRITE, &ctx)
            .await
            .unwrap()
            .unwrap_err();
        let PluginError::ValidationError(text) = err else {
            panic!("应为校验错误");
        };
        let parsed: VdfsValidationError = serde_json::from_str(&text).expect("应为结构化校验载荷");
        assert_eq!(parsed.fields[0].field, "text");
    }

    #[tokio::test]
    async fn delete_mkdir_and_mount_root_guards() {
        let (root, _) = roots();

        let ctx = ctx_with(json!({ "path": "/mem/sub", "recursive": true }));
        assert!(dispatch(&root, VFDS_DELETE, &ctx).await.unwrap().is_ok());

        let ctx = ctx_with(json!({ "path": "/mem" }));
        assert!(matches!(
            dispatch(&root, VFDS_DELETE, &ctx).await.unwrap(),
            Err(PluginError::Forbidden(_))
        ));

        let ctx = ctx_with(json!({ "path": "/mem" }));
        assert!(matches!(
            dispatch(&root, VFDS_MKDIR, &ctx).await.unwrap(),
            Err(PluginError::ValidationError(_))
        ));
    }

    #[tokio::test]
    async fn mount_root_is_not_readable() {
        let (root, _) = roots();
        let ctx = ctx_with(json!({ "path": "/mem" }));
        let err = dispatch(&root, VFDS_READ, &ctx).await.unwrap().unwrap_err();
        assert!(matches!(err, PluginError::Forbidden(_)));
    }

    /// 同挂载点内移动：全路径进、相对路径到 provider
    #[tokio::test]
    async fn same_mount_move_is_forwarded() {
        let (root, rec) = roots();
        let ctx = ctx_with(json!({ "from": "/mem/a.txt", "to": "/mem/b.txt" }));
        let resp = dispatch(&root, VFDS_MOVE, &ctx).await.unwrap().unwrap();
        let m = resp.get::<VdfsMoveResponse>().unwrap();
        assert_eq!(m.from, "/mem/a.txt");
        assert_eq!(m.to, "/mem/b.txt");
        assert!(
            rec.seen().is_empty(),
            "move 不触达 provider（Rec 未记录 move）"
        );
    }

    /// 跨挂载点移动由根拒绝（访问层不做拓扑判定）
    #[tokio::test]
    async fn cross_mount_move_is_rejected() {
        let rec = Rec::new();
        let a: DynVdfsProvider = rec.clone();
        let b: DynVdfsProvider = Arc::new(Bare);
        let root: DynVdfsProvider = Arc::new(VdfsMountTable::new(vec![
            ("mem".into(), a),
            ("bare".into(), b),
        ]));
        let ctx = ctx_with(json!({ "from": "/mem/a.txt", "to": "/bare/a.txt" }));
        assert!(matches!(
            dispatch(&root, VFDS_MOVE, &ctx).await.unwrap(),
            Err(PluginError::ValidationError(_))
        ));
    }

    #[tokio::test]
    async fn watch_unwatch_forward_full_path() {
        let (root, rec) = roots();
        let ctx = ctx_with(json!({ "path": "/mem/sub" }));
        assert!(dispatch(&root, VFDS_WATCH, &ctx).await.unwrap().is_ok());
        assert!(dispatch(&root, VFDS_UNWATCH, &ctx).await.unwrap().is_ok());
        assert_eq!(rec.seen(), vec!["sub", "sub"], "provider 收到相对路径");
    }

    /// 树遍历从根均匀展开：挂载点本身也是树的一层
    #[tokio::test]
    async fn tree_walks_uniformly_from_root() {
        let (root, _) = roots();
        let ctx = ctx_with(json!({ "path": "/" }));
        let t = dispatch(&root, VFDS_TREE, &ctx)
            .await
            .unwrap()
            .unwrap()
            .get::<VdfsTreeResponse>()
            .unwrap();
        let paths: Vec<&str> = t.nodes.iter().map(|n| n.path.as_str()).collect();
        assert_eq!(
            paths,
            vec!["/mem", "/mem/a.txt", "/mem/sub", "/mem/sub/b.md"]
        );
        assert!(!t.truncated);
    }

    #[tokio::test]
    async fn tree_respects_depth_and_limit() {
        let (root, _) = roots();

        let ctx = ctx_with(json!({ "path": "/mem", "depth": 1 }));
        let t = dispatch(&root, VFDS_TREE, &ctx)
            .await
            .unwrap()
            .unwrap()
            .get::<VdfsTreeResponse>()
            .unwrap();
        assert_eq!(
            t.nodes.iter().map(|n| n.path.as_str()).collect::<Vec<_>>(),
            vec!["/mem/a.txt", "/mem/sub"],
            "depth=1 → 只到直接子节点"
        );

        let ctx = ctx_with(json!({ "path": "/", "limit": 2 }));
        let t = dispatch(&root, VFDS_TREE, &ctx)
            .await
            .unwrap()
            .unwrap()
            .get::<VdfsTreeResponse>()
            .unwrap();
        assert!(t.truncated);
        assert_eq!(t.nodes.len(), 2);
    }

    /// 变更事件：provider 的相对路径已被根拼成全路径，访问层只取回顾首段
    #[test]
    fn change_event_derives_mount_from_full_path() {
        let e = to_change_event(&VdfsChange::new("/mem/sub/x.md", VFDS_CHANGE_UPDATED));
        assert_eq!(e.mount, "mem");
        assert_eq!(e.path, "/mem/sub/x.md");
        assert!(e.to.is_none());

        let r = to_change_event(&VdfsChange::renamed("/mem/a.txt", "/mem/b.txt"));
        assert_eq!(r.change, VFDS_CHANGE_RENAMED);
        assert_eq!(r.mount, "mem");
        assert_eq!(r.path, "/mem/a.txt");
        assert_eq!(r.to.as_deref(), Some("/mem/b.txt"));

        // 未带前导斜杠也能归一
        let n = to_change_event(&VdfsChange::new("x.md", VFDS_CHANGE_CREATED));
        assert_eq!(n.path, "/x.md");
    }

    // ==================== 根解析 ====================

    /// 无能力管理器、无父插件 → 空文件系统（可列出但无内容）
    #[tokio::test]
    async fn resolve_root_degrades_to_empty_vfs() {
        let root = resolve_root(None, &ctx_empty()).await;

        let resp = dispatch(&root, VFDS_PROVIDERS, &ctx_empty())
            .await
            .unwrap()
            .unwrap();
        assert!(resp
            .get::<VdfsProvidersResponse>()
            .unwrap()
            .providers
            .is_empty());

        let ctx = ctx_with(json!({ "path": "/" }));
        let data = dispatch(&root, VFDS_LIST, &ctx)
            .await
            .unwrap()
            .unwrap()
            .get::<VdfsListResponse>()
            .unwrap();
        assert_eq!(data.path, "/");
        assert!(data.items.is_empty());

        // 具体路径一律 NotFound
        let ctx = ctx_with(json!({ "path": "/mem" }));
        assert!(matches!(
            dispatch(&root, VFDS_LIST, &ctx).await.unwrap(),
            Err(PluginError::NotFound(_))
        ));
    }

    /// `ctx` 已带能力管理器 → 直接读其中的根（LLM 链路）
    #[tokio::test]
    async fn resolve_root_prefers_visitor_slot() {
        let rec = Rec::new();
        let visitor: Arc<dyn CapabilityVisitor> = Arc::new(DefaultToolVisitor::new());
        visitor.register_vdfs_root(root_with(&rec)).await;

        let ctx: Arc<dyn InvokeRequest> = Arc::new(SimpleRequest::new(None, None));
        ctx.set(CAPABILITY_VISITOR, visitor);

        let root = resolve_root(None, &ctx).await;
        let data = dispatch(&root, VFDS_LIST, &ctx_with(json!({ "path": "/mem" })))
            .await
            .unwrap()
            .unwrap()
            .get::<VdfsListResponse>()
            .unwrap();
        assert_eq!(data.node.name, "mem");
        assert_eq!(data.items.len(), 2);
    }

    /// 无能力管理器 → 从父插件广播，容器在广播中注册根（前端链路）
    #[tokio::test]
    async fn resolve_root_broadcasts_to_parent() {
        struct FakeContainer;

        #[async_trait]
        impl Plugin for FakeContainer {
            fn meta(&self) -> PluginMeta {
                PluginMeta::new("fake", "假容器")
            }

            async fn route(
                self: Arc<Self>,
                _ctx: Arc<dyn InvokeRequest>,
            ) -> InvokeResponse<PluginPayload> {
                Err(PluginError::NotFound("fake".into()))
            }

            async fn traverse(
                self: Arc<Self>,
                _path: String,
                ctx: Arc<dyn InvokeRequest>,
            ) -> InvokeResponse<PluginPayload> {
                if ctx.get(PATH).as_deref() == Some(TRAVERSE_AVAILABLE_TOOLS) {
                    if let Some(v) = ctx.get(CAPABILITY_VISITOR) {
                        v.register_vdfs_root(root_with(&Rec::new())).await;
                    }
                }
                Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
            }
        }

        let parent: Arc<dyn Plugin> = Arc::new(FakeContainer);
        let root = resolve_root(Some(&parent), &ctx_empty()).await;

        let ctx = ctx_with(json!({ "path": "/" }));
        let items = dispatch(&root, VFDS_LIST, &ctx)
            .await
            .unwrap()
            .unwrap()
            .get::<VdfsListResponse>()
            .unwrap()
            .items;
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "mem");
        assert_eq!(items[0].path, "/mem");
    }
}
