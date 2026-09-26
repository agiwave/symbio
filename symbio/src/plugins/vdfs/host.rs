//! VDFS 访问层（vdfs 插件侧）—— 只做「地址 + 信封」的机械转发
//!
//! ## 本层职责（就这两件）
//!
//! 1. **前端链路**：把 `vdfs/*` 协议请求翻译成对**统一文件系统**（[`UnifiedFs`]）
//!    的调用，结果装进线路信封（[`super::protocol`]）；
//! 2. **LLM 链路**：同一份翻译经 [`super::tools`] 暴露为工具。
//!
//! ```text
//! 前端 / LLM ──vdfs/*──▶ 本层（拆信封）──▶ UnifiedFs（地址分流）──▶ 虚拟层 / 物理层
//! ```
//!
//! ## 本层不认识目录拓扑
//!
//! 虚拟层根归**组合容器**所有：容器把自己的组合视图注册为 VDFS 根
//! （`CapabilityVisitor::register_vdfs_root`，见 `plugins/composite/vdfs.rs`）。
//! 本层只取这个根、交给门面，把**规范化后的地址**原样递过去——
//! 根之下有多少类别、叫什么，本层完全不知道、也不必知道。
//! 于是「新增资源 = 新增一个注册」不需要改动本层任何一行。
//!
//! 地址规则（根名前缀 = 虚拟，其余 = 磁盘）也不在本层：它在 [`UnifiedFs`]，
//! 两条链路共用同一个实例；根名本身只归 [`super::fs::VDFS_ADDR_ROOT`] 一处。
//!
//! ## 根从哪来
//!
//! - **LLM 链路**：`ctx` 已携带 `CAPABILITY_VISITOR`（编排方 `attach_capabilities`
//!   注入），容器在那次广播里把根注册了进去，直接 `get_vdfs_root()` 即可；
//! - **前端链路**：`ctx` 无能力管理器，从父插件广播一次
//!   `traverse(TRAVERSE_AVAILABLE_TOOLS)`，容器在广播中把根注册进新收集器。
//!
//! 两条链路取到的是**同一个根**。取不到（无组合容器）时虚拟层降级为**空目录**
//! 而非报错——「系统没有资源」与「资源为空」表现一致，前端与 LLM 都不必特判；
//! 物理层与它无关，照常可用。
//!
//! [`UnifiedFs`]: super::fs::UnifiedFs

pub use super::fs::{normalize_addr, UnifiedFs, VDFS_ADDR_ROOT};
use super::protocol::*;
use crate::symbio_core::vdfs_context;
use crate::symbio_core::EVENT_BUS_KIND_VDFS;
use crate::symbio_core::{
    vdfs_derive_ext, vdfs_has_parent_segment, DynVdfsProvider, VdfsAccess, VdfsChange,
    VdfsChangeSink, VdfsContent, VdfsContext, VdfsError, VdfsItem, VdfsNode, VdfsParams,
    VdfsProvider, VdfsResult, VdfsWriteResponse, VDFS_PARAM_BEFORE, VDFS_PARAM_LIMIT,
    VDFS_PARAM_WORKDIR,
};
use crate::symbio_core::{
    CapabilityVisitor, Plugin, PluginInvokeRequest, PluginInvokeRequestExt, PluginInvokeResponse,
    PluginPayload, CAPABILITY_VISITOR, WORKDIR,
};
use crate::symbio_core::{VdfsRequest, VdfsResponse};
use crate::symbio_core::{ROUTE_VDFS_ROOT, ROUTE_VDFS_UNWATCH, ROUTE_VDFS_WATCH};
use async_trait::async_trait;
use std::collections::VecDeque;
use std::sync::Arc;

// 变更事件的 `kind` 不自持：取自词表的家 `symbio_core::event_bus::EVENT_BUS_KIND_VDFS`
// （前端 `subscribe({ kind: 'vdfs' })`，见 PROTOCOLS.md §事件总线频道）。

// ==================== 统一文件系统 ====================

/// 虚拟层降级用的空服务者（无容器登记时）。
///
/// 自身目录可列出（内容为空），其余虚拟地址一律 `NotFound`——语义与「有容器但没有
/// 资源」完全一致，消费者无需为「没有容器」写第二条分支。
struct EmptyVdfs;

#[async_trait]
impl VdfsProvider for EmptyVdfs {
    async fn dispatch(
        &self,
        _ctx: &VdfsContext,
        path: &str,
        req: VdfsRequest,
    ) -> VdfsResult<VdfsResponse> {
        match req {
            VdfsRequest::List { .. } => {
                if path.is_empty() {
                    Ok(VdfsResponse::list(Vec::<VdfsNode>::new()))
                } else {
                    Err(VdfsError::not_found(path))
                }
            }
            VdfsRequest::Stat => {
                if path.is_empty() {
                    Ok(VdfsResponse::Stat(VdfsNode::dir(
                        "",
                        "系统",
                        VdfsAccess::LIST_TRAVERSE,
                    )))
                } else {
                    Err(VdfsError::not_found(path))
                }
            }
            _ => Err(VdfsError::not_found(path)),
        }
    }
}

/// 虚拟层降级用的空根
pub fn empty_root() -> DynVdfsProvider {
    Arc::new(EmptyVdfs)
}

/// 从能力管理器取容器注册的虚拟层根；未注册时降级为空根。
///
/// 调用方需已确保 `visitor` 存在（LLM 链路里它是硬前提，缺失应显式报错）。
pub async fn root_of(visitor: &Arc<dyn CapabilityVisitor>) -> DynVdfsProvider {
    visitor.get_vdfs_root().await.unwrap_or_else(empty_root)
}

/// 构造本次调用的统一文件系统（虚拟层 + 物理层）。
pub async fn unified_fs(visitor: &Arc<dyn CapabilityVisitor>) -> DynVdfsProvider {
    Arc::new(UnifiedFs::new(root_of(visitor).await))
}

/// 取本次调用的统一文件系统（前端链路入口）。
///
/// - `ctx` 已带能力管理器（LLM 链路）→ 直接用其中的根（`CapabilityVisitor` 收集）；
/// - 否则（前端链路）→ 经 core 的 `Plugin::get_vfs_provider` **直接取**父插件暴露的
///   虚拟层根，不广播、不依赖 `CapabilityVisitor`。两条链路拿到的是同一个根实例。
pub async fn resolve_fs(
    parent: Option<&Arc<dyn Plugin>>,
    ctx: &Arc<dyn PluginInvokeRequest>,
) -> DynVdfsProvider {
    if let Some(visitor) = ctx.get(CAPABILITY_VISITOR) {
        return unified_fs(&visitor).await;
    }

    // 前端链路：系统 vfs provider 经 `Plugin` 接口直接取（与 LLM 链路的
    // `CapabilityVisitor` 无关），取不到时降级为空根。
    let root = match parent {
        Some(p) => Arc::clone(p).get_vfs_provider().unwrap_or_else(empty_root),
        None => empty_root(),
    };
    Arc::new(UnifiedFs::new(root))
}

// ==================== 变更投递 ====================

/// 统一文件系统报出的变更 → 总线事件。
///
/// 门面已把事件里的路径补成对外展示地址（`.vdfsv2/<类别>/…`），与消费者请求时用的
/// 坐标系一致，因此本层只是换个信封投到总线上，不再做任何路径加工。
///
/// **`delta` 原样带过**：它是**正文**不是路径，门面不该碰它（逐字段重建会把它丢掉，
/// 且没有编译错误提示）。这也正是 `VdfsChange::map_paths` 存在的理由。
/// 构造变更投递器：接到全局事件总线，下发前端（`kind = EVENT_BUS_KIND_VDFS`）。
fn event_bus_sink() -> VdfsChangeSink {
    // 信封与 provider 侧形状重合（本层只补 `map_paths` 挂载名），**原样**投上总线——
    // 不再有一个「换信封」的翻译层（那层曾叫 `to_change_event` → `VdfsChangeEvent`，
    // 两者形状逐字相同，纯复制；S27 合并删除）。
    Arc::new(move |change: VdfsChange| {
        let data = serde_json::to_value(&change).unwrap_or(serde_json::Value::Null);
        crate::symbio_core::EventBus::try_publish(EVENT_BUS_KIND_VDFS, None, data);
    })
}

// ==================== 请求载荷 ====================

/// 读取请求载荷（缺省容忍空载荷）
fn payload_or_default<T: serde::de::DeserializeOwned + Default + Clone + Send + Sync + 'static>(
    ctx: &Arc<dyn PluginInvokeRequest>,
) -> T {
    ctx.payload::<serde_json::Value>()
        .ok()
        .and_then(|v| serde_json::from_value::<T>(v).ok())
        .unwrap_or_default()
}

// ==================== 调用级参数 ====================

/// 宿主 ctx → VDFS 调用级参数：把运行时状态翻译成 provider 的**约定键**。
///
/// 这里是唯一的翻译点——provider 只认 [`VDFS_PARAM_WORKDIR`] 这类约定键，
/// 不认识宿主 ctx 的键名（`WORKDIR` 等）。两条链路共用：前端协议入口与 LLM 工具
/// 都把 workdir 送到同一个键上，物理层据此解析相对地址。
pub fn call_params(ctx: &Arc<dyn PluginInvokeRequest>) -> VdfsParams {
    let mut params = VdfsParams::new();
    if let Some(workdir) = ctx.get(WORKDIR) {
        params.insert(
            VDFS_PARAM_WORKDIR.to_string(),
            serde_json::Value::String(workdir),
        );
    }
    params
}

// ==================== 地址回填（兜底） ====================

/// 子节点地址（`base` = 父目录地址）。
///
/// 地址是**展示口径**的：`base` 为空即工作目录根，子节点直接是裸名字
/// （`README.md`）；虚拟根下则是 `<根>/<名字>`。两条链路同一形态——
/// 父地址从请求里来，因此这里不需要知道根叫什么。
fn child_path(base: &str, name: &str) -> String {
    let base = base.trim_end_matches('/');
    if base.is_empty() {
        name.to_string()
    } else {
        format!("{base}/{name}")
    }
}

/// 条目在**本次列表**里的地址。
///
/// provider 没填就按 `<父地址>/<name>` 推导——[`VdfsNode::name`] 就是父节点内的
/// 路径段，所以这条推导总是成立；provider 填了（地址不在本目录下，如设置页条目
/// 指向插件自己的配置文档）就用它填的。这条规则在**一处**实现，`list` / `tree` /
/// `search` 三处消费共用——各写一份必然各漏一份。
fn item_addr(base: &str, item: &VdfsItem) -> String {
    if item.path.is_empty() {
        child_path(base, &item.node.name)
    } else {
        item.path.clone()
    }
}

/// 兜底回填：条目地址按 [`item_addr`] 补，节点上的 `ext` / `title` 按 `name` 补。
///
/// 门面与虚拟层根本身已回填，这里是**协议通用兜底**——任何 provider 实现都可以
/// 只填 `name` 与 `access`，其余由访问层补全。
fn fill_paths(base: &str, items: &mut [VdfsItem]) {
    for it in items.iter_mut() {
        it.path = item_addr(base, it);
        if it.node.ext.is_none() {
            it.node.ext = vdfs_derive_ext(&it.node.name);
        }
        if it.node.title.is_empty() {
            it.node.title = it.node.name.clone();
        }
    }
}

/// 目录自身节点的兜底（provider 未实现 `stat` 时）
fn dir_self(addr: &str) -> VdfsNode {
    let name = if addr.is_empty() {
        ".".to_string()
    } else {
        addr.rsplit('/').next().unwrap_or(addr).to_string()
    };
    VdfsNode::dir(name.clone(), name, VdfsAccess::dir(true, true))
}

// ==================== 统一分发 ====================

/// `vdfs/*` 统一分发入口。
///
/// `fs` 是**统一文件系统**（[`resolve_fs`] / [`unified_fs`] 的产物）：本层不认识
/// 它背后的虚拟层与物理层，只把地址原样递过去。
///
/// 返回 `None` 表示该 path 不是 VDFS 协议路径（调用方继续自己的 match）：
///
/// ```ignore
/// if let Some(resp) = host::dispatch_with(&fs, path, &ctx, params).await {
///     return resp;
/// }
/// ```
///
/// `params` 是**调用级参数**：调用方把请求 ctx 里的运行时状态（如 workdir）按约定键
/// 透传给 provider，provider 因此不必知道宿主 ctx 的键名约定（见 [`VDFS_PARAM_WORKDIR`]）。
/// 不需要任何参数时传空的 [`VdfsParams`]。
pub async fn dispatch_with(
    fs: &DynVdfsProvider,
    path: &str,
    ctx: &Arc<dyn PluginInvokeRequest>,
    params: VdfsParams,
) -> Option<PluginInvokeResponse<PluginPayload>> {
    if !VDFS_OPS.contains(&path) {
        return None;
    }
    let vctx = vdfs_context(ctx).with_params(params);
    let resp = match path {
        ROUTE_VDFS_ROOT => root(fs, &vctx, ctx).await,
        VDFS_LIST => list(fs, &vctx, ctx).await,
        VDFS_TREE => tree(fs, &vctx, ctx).await,
        VDFS_STAT => stat(fs, &vctx, ctx).await,
        VDFS_READ => read(fs, &vctx, ctx).await,
        VDFS_WRITE => write(fs, &vctx, ctx).await,
        VDFS_DELETE => delete(fs, &vctx, ctx).await,
        VDFS_MKDIR => mkdir(fs, &vctx, ctx).await,
        VDFS_EDIT => edit(fs, &vctx, ctx).await,
        VDFS_SEARCH => search(fs, &vctx, ctx).await,
        ROUTE_VDFS_WATCH | ROUTE_VDFS_UNWATCH => {
            watch(fs, &vctx, ctx, path == ROUTE_VDFS_WATCH).await
        }
        VDFS_ACTION => action(fs, &vctx, ctx).await,
        _ => unreachable!("VDFS_OPS 与分发分支必须一一对应"),
    };
    Some(resp)
}

/// `vdfs/root` —— **进入地址空间**：列出虚拟根，调用方不给地址。
///
/// 地址由宿主填上**自己挂的那个名字**（[`VDFS_ADDR_ROOT`]）——消费方因此不需要知道
/// 它，拿回包里的 `path` 当运行期数据即可。除此之外与 `list` 完全同一条路径
/// （同一个 `list_at`，同一套回填与展示口径）。
async fn root(
    fs: &DynVdfsProvider,
    vctx: &VdfsContext,
    ctx: &Arc<dyn PluginInvokeRequest>,
) -> PluginInvokeResponse<PluginPayload> {
    // 请求载荷忽略：本操作的定义就是「不给地址」（给了也不看，免得出现两套入参）
    let _ = payload_or_default::<VdfsPathRequest>(ctx);
    list_at(fs, vctx, VDFS_ADDR_ROOT.to_string(), None, None).await
}

async fn list(
    root: &DynVdfsProvider,
    vctx: &VdfsContext,
    ctx: &Arc<dyn PluginInvokeRequest>,
) -> PluginInvokeResponse<PluginPayload> {
    let req: VdfsPathRequest = payload_or_default(ctx);
    let addr = normalize_addr(&req.path)?;
    let before = req.before.filter(|b| !b.is_empty());
    list_at(root, vctx, addr, req.limit, before).await
}

/// 列表的公共体 —— `list` 与 `root` 只差「地址从哪来」。
async fn list_at(
    root: &DynVdfsProvider,
    vctx: &VdfsContext,
    addr: String,
    limit: Option<u32>,
    before: Option<String>,
) -> PluginInvokeResponse<PluginPayload> {
    // 有界列表：把窗口参数放进**调用级参数袋**再分发。
    // 之所以用参数袋、而不是给 `VdfsProvider::list` 加参数，是为了让「不认识窗口」
    // 的 provider **完全不受影响**——它们不取这两个键，行为与从前逐字节一致。
    let vctx = {
        let mut c = vctx.clone();
        if let Some(limit) = limit {
            c = c.with_param(VDFS_PARAM_LIMIT, limit);
        }
        if let Some(before) = before.as_deref() {
            c = c.with_param(VDFS_PARAM_BEFORE, before);
        }
        c
    };

    let mut items = root
        .dispatch(
            &vctx,
            &addr,
            VdfsRequest::List {
                limit,
                before: before.clone(),
            },
        )
        .await?
        .into_list()
        .ok_or_else(|| VdfsError::internal("provider 响应类型不匹配"))?;
    fill_paths(&addr, &mut items);

    // 目录自身节点：provider 未实现 stat 时按目录形态兜底
    let node = match root.dispatch(&vctx, &addr, VdfsRequest::Stat).await {
        Ok(resp) => match resp.into_stat() {
            Some(mut n) => {
                if n.title.is_empty() {
                    n.title = n.name.clone();
                }
                if n.ext.is_none() {
                    n.ext = vdfs_derive_ext(&n.name);
                }
                n
            }
            None => dir_self(&addr),
        },
        Err(_) => dir_self(&addr),
    };

    Ok(PluginPayload::new(&VdfsListResponse {
        path: addr,
        node,
        items,
    }))
}

async fn stat(
    root: &DynVdfsProvider,
    vctx: &VdfsContext,
    ctx: &Arc<dyn PluginInvokeRequest>,
) -> PluginInvokeResponse<PluginPayload> {
    let req: VdfsPathRequest = payload_or_default(ctx);
    let addr = normalize_addr(&req.path)?;
    let mut n = root
        .dispatch(vctx, &addr, VdfsRequest::Stat)
        .await?
        .into_stat()
        .ok_or_else(|| VdfsError::internal("provider 响应类型不匹配"))?;
    if n.title.is_empty() {
        n.title = n.name.clone();
    }
    if n.ext.is_none() {
        n.ext = vdfs_derive_ext(&n.name);
    }
    Ok(PluginPayload::new(&n))
}

async fn read(
    root: &DynVdfsProvider,
    vctx: &VdfsContext,
    ctx: &Arc<dyn PluginInvokeRequest>,
) -> PluginInvokeResponse<PluginPayload> {
    let req: VdfsPathRequest = payload_or_default(ctx);
    let addr = normalize_addr(&req.path)?;
    let c = root
        .dispatch(vctx, &addr, VdfsRequest::Read)
        .await?
        .into_read()
        .ok_or_else(|| VdfsError::internal("provider 响应类型不匹配"))?;
    Ok(PluginPayload::new(&c))
}

async fn write(
    root: &DynVdfsProvider,
    vctx: &VdfsContext,
    ctx: &Arc<dyn PluginInvokeRequest>,
) -> PluginInvokeResponse<PluginPayload> {
    let req: VdfsWriteRequest = payload_or_default(ctx);
    let addr = normalize_addr(&req.path)?;
    // 机制级守卫：写入必须携带内容（语义级校验归 provider）
    if req.text.is_none() && req.b64.is_none() {
        return Err(VdfsError::invalid("写入需要 text 或 b64 之一作为内容").into());
    }
    let content = req.to_content();
    let r = root
        .dispatch(vctx, &addr, VdfsRequest::Write { content })
        .await?
        .into_write()
        .ok_or_else(|| VdfsError::internal("provider 响应类型不匹配"))?;
    Ok(PluginPayload::new(&r))
}

/// `vdfs/delete` —— 成功无产物（与 `watch` / `unwatch` 同形）。
///
/// 不回传被删地址：那是调用方给的（见 [`VdfsResponse`] 的文档）。
async fn delete(
    root: &DynVdfsProvider,
    vctx: &VdfsContext,
    ctx: &Arc<dyn PluginInvokeRequest>,
) -> PluginInvokeResponse<PluginPayload> {
    let req: VdfsPathRequest = payload_or_default(ctx);
    let addr = normalize_addr(&req.path)?;
    root.dispatch(
        vctx,
        &addr,
        VdfsRequest::Delete {
            recursive: req.recursive,
        },
    )
    .await?;
    Ok(PluginPayload::new(
        &crate::symbio_core::schemas::common::SuccessResponse::default(),
    ))
}

async fn mkdir(
    root: &DynVdfsProvider,
    vctx: &VdfsContext,
    ctx: &Arc<dyn PluginInvokeRequest>,
) -> PluginInvokeResponse<PluginPayload> {
    let req: VdfsPathRequest = payload_or_default(ctx);
    let addr = normalize_addr(&req.path)?;
    root.dispatch(vctx, &addr, VdfsRequest::Mkdir).await?;
    Ok(PluginPayload::new(&VdfsWriteResponse {
        name: None,
        created: true,
        etag: None,
    }))
}

/// `vdfs/action` —— 执行 provider 自持的节点动作（如「测试连接」）。
///
/// 本层不认识任何动作语义：只把 `(地址, 动作标识, 载荷)` 原样转发过去。
/// 动作是否存在、成功与否由对应的一层回答（未实现 → `NotImplemented`）。
async fn action(
    root: &DynVdfsProvider,
    vctx: &VdfsContext,
    ctx: &Arc<dyn PluginInvokeRequest>,
) -> PluginInvokeResponse<PluginPayload> {
    let req: VdfsActionRequest = payload_or_default(ctx);
    if req.action.trim().is_empty() {
        return Err(VdfsError::invalid("动作标识不能为空").into());
    }
    let addr = normalize_addr(&req.path)?;
    let res = root
        .dispatch(
            vctx,
            &addr,
            VdfsRequest::Action {
                action: req.action,
                payload: req.payload,
            },
        )
        .await?
        .into_action()
        .ok_or_else(|| VdfsError::internal("provider 响应类型不匹配"))?;
    Ok(PluginPayload::new(&res))
}

// ==================== 组合操作（edit / search）====================
//
// provider 只出**原子操作**（list / stat / read / write / delete / mkdir / action）；
// 组合逻辑在访问层**只写一次**——前端协议入口用下面的 handler，LLM 工具链路
// （`provider::ToolVdfs`）直接调用 `edit_via` / `search_via`，任何 `VdfsProvider`
// 实现方都无需重复实现这些逻辑。
//
// 注意**移动不在其中**：它不是组合操作而是被整条下线了——理由见
// `symbio_core::vdfs` 的「没有 `Move`」一节（跨子树时它不是原语，
// 由外层组合才是它的正确位置；当前外层也没提供）。

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

    let raw = provider
        .dispatch(vctx, rel, VdfsRequest::Read)
        .await?
        .into_read()
        .ok_or_else(|| VdfsError::internal("provider 响应类型不匹配"))?;
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
        .dispatch(
            vctx,
            rel,
            VdfsRequest::Write {
                content: VdfsContent::text(final_content),
            },
        )
        .await?;

    Ok(VdfsEditResponse {
        replaced: 1,
        message: Some(format!("已编辑 {rel}：替换了 1 处（{size} 字节）")),
    })
}

/// 单次搜索返回上限（与原生 `glob_search` 一致）
pub(crate) const MAX_SEARCH_RESULTS: usize = 1000;

/// 文件名 Glob 搜索 = 递归 `list` + 模式过滤的组合。
///
/// 沿 [`VdfsProvider::list`] 下钻（`t` 位控制、单分支失败跳过），对每个普通文件做
/// Glob 匹配；provider 的安全规则（访问位、路径守卫）经 `list` 自持生效。
///
/// `base` 为搜索基地址（空 = 工作目录根）。**模式相对于 `base`**，**结果与 `base`
/// 同坐标系**（即返回可直接再次寻址的完整地址）——两条链路、虚拟层与物理层都是
/// 这一条规则，因此 `*.rs` 在 `.vdfsv2/session` 下与在 `src` 下含义一致。
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
    // 与物理层、shell 策略共用同一条规则（按段判定，与分隔符无关）：
    // 原先的 `contains("../") || contains("..\\") || == ".."` 会放过 `a/..`
    // 这种 `..` 收尾的写法——三处各写一份，就必然三处各漏一处。
    if vdfs_has_parent_segment(pattern) {
        return Err(VdfsError::invalid(
            "不允许在 Glob 模式中使用路径遍历 ('..')。",
        ));
    }
    let pat = glob::Pattern::new(pattern)
        .map_err(|e| VdfsError::invalid(format!("无效的 Glob 模式：{e}")))?;

    let base_dir = base.trim_end_matches('/').to_string();
    let mut results: Vec<String> = Vec::new();
    let mut truncated = false;
    let mut queue: VecDeque<String> = VecDeque::new();
    queue.push_back(base.to_string());

    while let Some(dir) = queue.pop_front() {
        let children = match provider
            .dispatch(
                vctx,
                &dir,
                VdfsRequest::List {
                    limit: None,
                    before: None,
                },
            )
            .await
        {
            Ok(resp) => resp
                .into_list()
                .ok_or_else(|| VdfsError::internal("provider 响应类型不匹配"))?,
            // 单分支失败降级：不让一棵子树拖垮整个搜索
            Err(e) => {
                crate::plugin_warn!("vdfs", "vdfs_search: 列出 {dir} 失败，已跳过: {e}");
                continue;
            }
        };
        // 子地址 = 父目录 + 子名（`""` 基目录直接用子名，保持相对形态）；
        // provider 显式给了地址的条目（如设置页条目指向别的挂载点）用它给的那个。
        let parent = dir.trim_end_matches('/').to_string();
        for child in children {
            let child_addr = item_addr(&parent, &child);
            // 匹配用「相对 base 的地址」，收集用完整地址
            let rel = child_addr
                .strip_prefix(base_dir.as_str())
                .map(|s| s.trim_start_matches('/'))
                .unwrap_or(child_addr.as_str());
            if !child.node.is_dir() && pat.matches(rel) {
                results.push(child_addr.clone());
                if results.len() >= MAX_SEARCH_RESULTS {
                    truncated = true;
                    break;
                }
            }
            if child.node.access.traverse {
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
    ctx: &Arc<dyn PluginInvokeRequest>,
) -> PluginInvokeResponse<PluginPayload> {
    let req: VdfsEditRequest = payload_or_default(ctx);
    let addr = normalize_addr(&req.path)?;
    let r = edit_via(root, vctx, &addr, &req.old_string, &req.new_string).await?;
    Ok(PluginPayload::new(&r))
}

async fn search(
    root: &DynVdfsProvider,
    vctx: &VdfsContext,
    ctx: &Arc<dyn PluginInvokeRequest>,
) -> PluginInvokeResponse<PluginPayload> {
    let req: VdfsSearchRequest = payload_or_default(ctx);
    let addr = normalize_addr(&req.path)?;
    let r = search_via(root, vctx, &addr, &req.pattern).await?;
    Ok(PluginPayload::new(&r))
}

async fn watch(
    root: &DynVdfsProvider,
    vctx: &VdfsContext,
    ctx: &Arc<dyn PluginInvokeRequest>,
    subscribe: bool,
) -> PluginInvokeResponse<PluginPayload> {
    let req: VdfsPathRequest = payload_or_default(ctx);
    let addr = normalize_addr(&req.path)?;
    if subscribe {
        root.dispatch(
            vctx,
            &addr,
            VdfsRequest::Watch {
                sink: event_bus_sink(),
            },
        )
        .await?;
    } else {
        root.dispatch(vctx, &addr, VdfsRequest::Unwatch).await?;
    }
    Ok(PluginPayload::new(
        &crate::symbio_core::schemas::common::SuccessResponse::default(),
    ))
}

/// 树状遍历：访问层统一实现（递归 [`VdfsProvider::list`]）。
///
/// provider 只需实现 `list`，并在目录节点的 `access` 上声明 `t` 位即可被遍历；
/// 机制不含任何场景语义。这里**不需要认识目录拓扑**——地址本身就是递归的地址。
/// 深度与数量上限防爆炸，超限时 `truncated = true`。
async fn tree(
    root: &DynVdfsProvider,
    vctx: &VdfsContext,
    ctx: &Arc<dyn PluginInvokeRequest>,
) -> PluginInvokeResponse<PluginPayload> {
    let req: VdfsTreeRequest = payload_or_default(ctx);
    let addr = normalize_addr(&req.path)?;
    let depth_limit = req.depth.unwrap_or(3); // 0 = 不限
    let count_limit = req.limit.unwrap_or(500).max(1) as usize;

    let mut out: Vec<VdfsItem> = Vec::new();
    let mut truncated = false;

    // 队列元素 = (目录地址, 深度)
    let mut queue: VecDeque<(String, u32)> = VecDeque::new();
    queue.push_back((addr.clone(), 0));

    while let Some((dir, depth)) = queue.pop_front() {
        let mut children = match root
            .dispatch(
                vctx,
                &dir,
                VdfsRequest::List {
                    limit: None,
                    before: None,
                },
            )
            .await
        {
            Ok(resp) => resp
                .into_list()
                .ok_or_else(|| VdfsError::internal("provider 响应类型不匹配"))?,
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
            let descend =
                child.node.access.traverse && (depth_limit == 0 || depth + 1 < depth_limit);
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
        nodes: out,
        truncated,
    }))
}

#[cfg(test)]
#[path = "host.test.rs"]
mod tests;
