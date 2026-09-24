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
use crate::symbio_core::event_bus::KIND_VDFS;
use crate::symbio_core::vdfs::vdfs_context;
use crate::symbio_core::vdfs_provider::*;
use crate::symbio_core::vdfs_provider::{VdfsRequest, VdfsResponse};
use crate::symbio_core::{
    CapabilityVisitor, InvokeRequest, InvokeRequestExt, InvokeResponse, Plugin, PluginPayload,
    CAPABILITY_VISITOR, WORKDIR,
};
use async_trait::async_trait;
use std::collections::VecDeque;
use std::sync::Arc;

// 变更事件的 `kind` 不自持：取自词表的家 `symbio_core::event_bus::KIND_VDFS`
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
                    Ok(VdfsResponse::List(Vec::new()))
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
    ctx: &Arc<dyn InvokeRequest>,
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
/// 构造变更投递器：接到全局事件总线，下发前端（`kind = KIND_VDFS`）。
fn event_bus_sink() -> VdfsChangeSink {
    // 信封与 provider 侧形状重合（本层只补 `map_paths` 挂载名），**原样**投上总线——
    // 不再有一个「换信封」的翻译层（那层曾叫 `to_change_event` → `VdfsChangeEvent`，
    // 两者形状逐字相同，纯复制；S27 合并删除）。
    Arc::new(move |change: VdfsChange| {
        let data = serde_json::to_value(&change).unwrap_or(serde_json::Value::Null);
        crate::symbio_core::event_bus::EventBus::try_publish(KIND_VDFS, None, data);
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
/// 这里是唯一的翻译点——provider 只认 [`VDFS_PARAM_WORKDIR`] 这类约定键，
/// 不认识宿主 ctx 的键名（`WORKDIR` 等）。两条链路共用：前端协议入口与 LLM 工具
/// 都把 workdir 送到同一个键上，物理层据此解析相对地址。
pub fn call_params(ctx: &Arc<dyn InvokeRequest>) -> VdfsParams {
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

/// 兜底回填：provider 未填 `path` / `ext` / `title` 时按请求地址补齐。
///
/// 门面与虚拟层根本身已回填，这里是**协议通用兜底**——任何 provider 实现都可以
/// 只填 `name` 与 `access`，其余由访问层补全。
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
fn dir_self(addr: &str) -> VdfsNode {
    let name = if addr.is_empty() {
        ".".to_string()
    } else {
        addr.rsplit('/').next().unwrap_or(addr).to_string()
    };
    let mut n = VdfsNode::dir(name.clone(), name, VdfsAccess::dir(true, true));
    n.path = addr.to_string();
    n
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
    ctx: &Arc<dyn InvokeRequest>,
    params: VdfsParams,
) -> Option<InvokeResponse<PluginPayload>> {
    if !VDFS_OPS.contains(&path) {
        return None;
    }
    let vctx = vdfs_context(ctx).with_params(params);
    let resp = match path {
        VDFS_ROOT => root(fs, &vctx, ctx).await,
        VDFS_LIST => list(fs, &vctx, ctx).await,
        VDFS_TREE => tree(fs, &vctx, ctx).await,
        VDFS_STAT => stat(fs, &vctx, ctx).await,
        VDFS_READ => read(fs, &vctx, ctx).await,
        VDFS_WRITE => write(fs, &vctx, ctx).await,
        VDFS_DELETE => delete(fs, &vctx, ctx).await,
        VDFS_MKDIR => mkdir(fs, &vctx, ctx).await,
        VDFS_MOVE => move_item(fs, &vctx, ctx).await,
        VDFS_EDIT => edit(fs, &vctx, ctx).await,
        VDFS_SEARCH => search(fs, &vctx, ctx).await,
        VDFS_WATCH | VDFS_UNWATCH => watch(fs, &vctx, ctx, path == VDFS_WATCH).await,
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
    ctx: &Arc<dyn InvokeRequest>,
) -> InvokeResponse<PluginPayload> {
    // 请求载荷忽略：本操作的定义就是「不给地址」（给了也不看，免得出现两套入参）
    let _ = payload_or_default::<VdfsPathRequest>(ctx);
    list_at(fs, vctx, VDFS_ADDR_ROOT.to_string(), None, None).await
}

async fn list(
    root: &DynVdfsProvider,
    vctx: &VdfsContext,
    ctx: &Arc<dyn InvokeRequest>,
) -> InvokeResponse<PluginPayload> {
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
) -> InvokeResponse<PluginPayload> {
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
                    n.ext = derive_ext(&n.name);
                }
                if n.path.is_empty() {
                    n.path = addr.clone();
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
    ctx: &Arc<dyn InvokeRequest>,
) -> InvokeResponse<PluginPayload> {
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
        n.ext = derive_ext(&n.name);
    }
    if n.path.is_empty() {
        n.path = addr;
    }
    Ok(PluginPayload::new(&n))
}

async fn read(
    root: &DynVdfsProvider,
    vctx: &VdfsContext,
    ctx: &Arc<dyn InvokeRequest>,
) -> InvokeResponse<PluginPayload> {
    let req: VdfsPathRequest = payload_or_default(ctx);
    let addr = normalize_addr(&req.path)?;
    let mut c = root
        .dispatch(vctx, &addr, VdfsRequest::Read)
        .await?
        .into_read()
        .ok_or_else(|| VdfsError::internal("provider 响应类型不匹配"))?;
    if c.path.is_empty() {
        c.path = addr;
    }
    Ok(PluginPayload::new(&c))
}

async fn write(
    root: &DynVdfsProvider,
    vctx: &VdfsContext,
    ctx: &Arc<dyn InvokeRequest>,
) -> InvokeResponse<PluginPayload> {
    let req: VdfsWriteRequest = payload_or_default(ctx);
    let addr = normalize_addr(&req.path)?;
    // 机制级守卫：写入必须携带内容（语义级校验归 provider）
    if req.text.is_none() && req.b64.is_none() {
        return Err(VdfsError::invalid("写入需要 text 或 b64 之一作为内容").into());
    }
    let content = req.to_content();
    let mut r = root
        .dispatch(vctx, &addr, VdfsRequest::Write { content })
        .await?
        .into_write()
        .ok_or_else(|| VdfsError::internal("provider 响应类型不匹配"))?;
    if r.path.is_empty() {
        r.path = addr;
    }
    Ok(PluginPayload::new(&r))
}

async fn delete(
    root: &DynVdfsProvider,
    vctx: &VdfsContext,
    ctx: &Arc<dyn InvokeRequest>,
) -> InvokeResponse<PluginPayload> {
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
    Ok(PluginPayload::new(&VdfsDeleteResponse { path: addr }))
}

async fn mkdir(
    root: &DynVdfsProvider,
    vctx: &VdfsContext,
    ctx: &Arc<dyn InvokeRequest>,
) -> InvokeResponse<PluginPayload> {
    let req: VdfsPathRequest = payload_or_default(ctx);
    let addr = normalize_addr(&req.path)?;
    root.dispatch(vctx, &addr, VdfsRequest::Mkdir).await?;
    Ok(PluginPayload::new(&VdfsWriteResponse {
        path: addr,
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
    ctx: &Arc<dyn InvokeRequest>,
) -> InvokeResponse<PluginPayload> {
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

async fn move_item(
    root: &DynVdfsProvider,
    vctx: &VdfsContext,
    ctx: &Arc<dyn InvokeRequest>,
) -> InvokeResponse<PluginPayload> {
    let req: VdfsMoveRequest = payload_or_default(ctx);
    let from = normalize_addr(&req.from)?;
    let to = normalize_addr(&req.to)?;
    // 「同一半内才可移动」由门面判定——本层只传地址
    root.dispatch(vctx, &from, VdfsRequest::Move { to: to.clone() })
        .await?;
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
        .dispatch(
            vctx,
            rel,
            VdfsRequest::Write {
                content: VdfsContent::text("", final_content),
            },
        )
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
    if has_parent_segment(pattern) {
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
        // 子地址 = 父目录 + 子名（`""` 基目录直接用子名，保持相对形态）
        let parent = dir.trim_end_matches('/').to_string();
        for child in children {
            let child_addr = if parent.is_empty() {
                child.name.clone()
            } else {
                format!("{parent}/{}", child.name)
            };
            // 匹配用「相对 base 的地址」，收集用完整地址
            let rel = child_addr
                .strip_prefix(base_dir.as_str())
                .map(|s| s.trim_start_matches('/'))
                .unwrap_or(child_addr.as_str());
            if !child.is_dir() && pat.matches(rel) {
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
    let addr = normalize_addr(&req.path)?;
    let r = edit_via(root, vctx, &addr, &req.old_string, &req.new_string).await?;
    Ok(PluginPayload::new(&r))
}

async fn search(
    root: &DynVdfsProvider,
    vctx: &VdfsContext,
    ctx: &Arc<dyn InvokeRequest>,
) -> InvokeResponse<PluginPayload> {
    let req: VdfsSearchRequest = payload_or_default(ctx);
    let addr = normalize_addr(&req.path)?;
    let r = search_via(root, vctx, &addr, &req.pattern).await?;
    Ok(PluginPayload::new(&r))
}

async fn watch(
    root: &DynVdfsProvider,
    vctx: &VdfsContext,
    ctx: &Arc<dyn InvokeRequest>,
    subscribe: bool,
) -> InvokeResponse<PluginPayload> {
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
    ctx: &Arc<dyn InvokeRequest>,
) -> InvokeResponse<PluginPayload> {
    let req: VdfsTreeRequest = payload_or_default(ctx);
    let addr = normalize_addr(&req.path)?;
    let depth_limit = req.depth.unwrap_or(3); // 0 = 不限
    let count_limit = req.limit.unwrap_or(500).max(1) as usize;

    let mut out: Vec<VdfsNode> = Vec::new();
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
        path: addr,
        nodes: out,
        truncated,
    }))
}

#[cfg(test)]
mod tests {
    use super::super::physical::PhysicalFs;
    use super::*;
    use crate::symbio_core::vdfs_provider::VDFS_KIND_DIR;
    use crate::symbio_core::{
        CapabilityVisitor, DefaultToolVisitor, PluginError, PluginMeta, SimpleRequest,
    };
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
        async fn dispatch(
            &self,
            _ctx: &VdfsContext,
            path: &str,
            req: VdfsRequest,
        ) -> VdfsResult<VdfsResponse> {
            match req {
                VdfsRequest::List { .. } => {
                    self.note(path);
                    Ok(VdfsResponse::List(match path {
                        "" => vec![
                            VdfsNode::file("a.txt", "A", VdfsAccess::READ_WRITE),
                            VdfsNode::dir("sub", "子目录", VdfsAccess::LIST_TRAVERSE),
                        ],
                        "sub" => vec![VdfsNode::file("b.md", "B", VdfsAccess::READ)],
                        _ => return Err(VdfsError::not_found(format!("无此目录：{path}"))),
                    }))
                }
                VdfsRequest::Stat => Ok(VdfsResponse::Stat(match path {
                    "a.txt" => VdfsNode::file("a.txt", "A", VdfsAccess::READ_WRITE),
                    "sub" => VdfsNode::dir("sub", "子目录", VdfsAccess::LIST_TRAVERSE),
                    "sub/b.md" => VdfsNode::file("b.md", "B", VdfsAccess::READ),
                    _ => return Err(VdfsError::not_found(path)),
                })),
                VdfsRequest::Read => {
                    self.note(path);
                    match path {
                        "a.txt" => Ok(VdfsResponse::Read(VdfsContent::text("", "hello"))),
                        "sub/b.md" => Ok(VdfsResponse::Read(VdfsContent::text("", "# b"))),
                        _ => Err(VdfsError::Forbidden("目录不可读".into())),
                    }
                }
                VdfsRequest::Write { content } => {
                    // 演示「provider 自持校验 + 字段级错误」
                    if content.text.as_deref() == Some("bad") {
                        return Err(VdfsError::Invalid(
                            VdfsValidationError::new("内容不合法").with_field("text", "不允许 bad"),
                        ));
                    }
                    Ok(VdfsResponse::Write(VdfsWriteResponse {
                        path: String::new(),
                        created: path == "new.txt",
                        etag: Some("v1".into()),
                    }))
                }
                VdfsRequest::Delete { .. } => {
                    if path == "sub" || path == "a.txt" {
                        Ok(VdfsResponse::Unit)
                    } else {
                        Err(VdfsError::Forbidden(format!("不允许删除 {path}")))
                    }
                }
                VdfsRequest::Action { action, .. } => {
                    self.note(&format!("action:{action}"));
                    Ok(VdfsResponse::Action(VdfsActionResult {
                        action: action.clone(),
                        ok: true,
                        message: format!("{path} 已执行 {action}"),
                        data: None,
                    }))
                }
                VdfsRequest::Move { to } => {
                    // 只接受相对路径：组合根已剥掉子目录前缀
                    if path == "a.txt" && to == "b.txt" {
                        Ok(VdfsResponse::Unit)
                    } else {
                        Err(VdfsError::Forbidden(format!("不支持移动 {path} → {to}")))
                    }
                }
                VdfsRequest::Watch { .. } => {
                    self.note(path);
                    Ok(VdfsResponse::Unit)
                }
                VdfsRequest::Unwatch => {
                    self.note(path);
                    Ok(VdfsResponse::Unit)
                }
                _ => Err(VdfsError::NotImplemented),
            }
        }
    }

    /// 极简 provider：验证「什么都不实现」也能被访问层容错
    struct Bare;

    #[async_trait]
    impl VdfsProvider for Bare {
        async fn dispatch(
            &self,
            _ctx: &VdfsContext,
            _path: &str,
            _req: VdfsRequest,
        ) -> VdfsResult<VdfsResponse> {
            Err(VdfsError::NotImplemented)
        }
    }

    /// 测试用 root 级 provider：首段 = 子目录名，委派给对应 provider（模拟 composite）
    struct TestRoot {
        dirs: Vec<(&'static str, DynVdfsProvider)>,
    }

    fn split_dir(path: &str) -> Option<(&str, &str)> {
        match path.find('/') {
            Some(i) => Some((&path[..i], &path[i + 1..])),
            None if path.is_empty() => None,
            None => Some((path, "")),
        }
    }

    impl TestRoot {
        fn resolve(&self, path: &str) -> VdfsResult<(&DynVdfsProvider, String)> {
            let (d, rel) =
                split_dir(path).ok_or_else(|| VdfsError::invalid("根目录不是可操作节点"))?;
            let p = self
                .dirs
                .iter()
                .find(|(n, _)| *n == d)
                .map(|(_, p)| p)
                .ok_or_else(|| {
                    let hint = self
                        .dirs
                        .iter()
                        .map(|(n, _)| *n)
                        .collect::<Vec<_>>()
                        .join(", ");
                    VdfsError::not_found(format!("目录不存在：{d}（现有：{hint}）"))
                })?;
            Ok((p, rel.to_string()))
        }
    }

    #[async_trait]
    impl VdfsProvider for TestRoot {
        async fn dispatch(
            &self,
            ctx: &VdfsContext,
            path: &str,
            req: VdfsRequest,
        ) -> VdfsResult<VdfsResponse> {
            // 根目录：可列 / 可 stat，其余拒绝
            let Some((d, _rel)) = split_dir(path) else {
                return match req {
                    VdfsRequest::List { .. } => Ok(VdfsResponse::List(
                        self.dirs
                            .iter()
                            .map(|(n, _p)| {
                                let mut x = VdfsNode::dir(
                                    n.to_string(),
                                    "内存子树",
                                    VdfsAccess::LIST_TRAVERSE,
                                );
                                x.path = n.to_string();
                                x
                            })
                            .collect(),
                    )),
                    VdfsRequest::Stat => Ok(VdfsResponse::Stat(VdfsNode::dir(
                        "",
                        "系统",
                        VdfsAccess::LIST_TRAVERSE,
                    ))),
                    _ => Err(VdfsError::invalid("根目录不是可操作节点")),
                };
            };
            let (p, rel) = self.resolve(path)?;
            let mismatch = || VdfsError::internal("响应类型不匹配");
            match req {
                VdfsRequest::List { .. } => {
                    let mut items = p
                        .dispatch(
                            ctx,
                            &rel,
                            VdfsRequest::List {
                                limit: None,
                                before: None,
                            },
                        )
                        .await?
                        .into_list()
                        .ok_or_else(mismatch)?;
                    for it in items.iter_mut() {
                        if it.path.is_empty() {
                            it.path = if rel.is_empty() {
                                format!("{d}/{}", it.name)
                            } else {
                                format!("{d}/{rel}/{}", it.name)
                            };
                        }
                    }
                    Ok(VdfsResponse::List(items))
                }
                VdfsRequest::Stat => {
                    if rel.is_empty() {
                        let mut n =
                            VdfsNode::dir(d.to_string(), "内存子树", VdfsAccess::LIST_TRAVERSE);
                        n.path = d.to_string();
                        return Ok(VdfsResponse::Stat(n));
                    }
                    let mut n = p
                        .dispatch(ctx, &rel, VdfsRequest::Stat)
                        .await?
                        .into_stat()
                        .ok_or_else(mismatch)?;
                    n.path = path.to_string();
                    Ok(VdfsResponse::Stat(n))
                }
                VdfsRequest::Read => {
                    if rel.is_empty() {
                        return Err(VdfsError::Forbidden(
                            "目录不是可读文件；请读取其子节点".to_string(),
                        ));
                    }
                    p.dispatch(ctx, &rel, VdfsRequest::Read).await
                }
                VdfsRequest::Write { content } => {
                    // 与生产容器同构：`rel` 为空 = 写在**挂载点目录自身**上（「新建」的
                    // 机制形态），原样转发；provider 生成的名字要补回树内路径。
                    let mut r = p
                        .dispatch(ctx, &rel, VdfsRequest::Write { content })
                        .await?
                        .into_write()
                        .ok_or_else(mismatch)?;
                    if r.path.is_empty() {
                        r.path = path.to_string();
                    } else if !r.path.starts_with(&format!("{d}/")) {
                        r.path = format!("{d}/{}", r.path);
                    }
                    Ok(VdfsResponse::Write(r))
                }
                VdfsRequest::Delete { recursive } => {
                    if rel.is_empty() {
                        return Err(VdfsError::Forbidden("目录不可删除".to_string()));
                    }
                    p.dispatch(ctx, &rel, VdfsRequest::Delete { recursive })
                        .await?;
                    Ok(VdfsResponse::Unit)
                }
                VdfsRequest::Mkdir => {
                    if rel.is_empty() {
                        return Err(VdfsError::invalid("目录已存在，无需创建"));
                    }
                    p.dispatch(ctx, &rel, VdfsRequest::Mkdir).await?;
                    Ok(VdfsResponse::Unit)
                }
                VdfsRequest::Move { to } => {
                    let (pt, rt) = self.resolve(&to)?;
                    if rel.is_empty() || rt.is_empty() {
                        return Err(VdfsError::Forbidden("目录不可移动".to_string()));
                    }
                    if std::sync::Arc::ptr_eq(p, pt) {
                        p.dispatch(ctx, &rel, VdfsRequest::Move { to: rt }).await?;
                        Ok(VdfsResponse::Unit)
                    } else {
                        Err(VdfsError::invalid("不支持跨目录移动"))
                    }
                }
                VdfsRequest::Action { action, payload } => {
                    p.dispatch(ctx, &rel, VdfsRequest::Action { action, payload })
                        .await
                }
                VdfsRequest::Watch { sink } => {
                    p.dispatch(ctx, &rel, VdfsRequest::Watch { sink }).await?;
                    Ok(VdfsResponse::Unit)
                }
                VdfsRequest::Unwatch => {
                    p.dispatch(ctx, &rel, VdfsRequest::Unwatch).await?;
                    Ok(VdfsResponse::Unit)
                }
            }
        }
    }

    /// 以 `rec` 为唯一子目录（mem）的虚拟层根（模拟 composite）
    fn root_with(rec: &Arc<Rec>) -> DynVdfsProvider {
        let p: DynVdfsProvider = rec.clone();
        Arc::new(TestRoot {
            dirs: vec![("mem", p)],
        })
    }

    /// 被测的统一文件系统：虚拟层 = mem 子目录，物理层 = 磁盘
    fn fs_roots() -> (DynVdfsProvider, Arc<Rec>) {
        let rec = Rec::new();
        (
            Arc::new(UnifiedFs::with_physical(
                root_with(&rec),
                Arc::new(PhysicalFs::new()),
            )),
            rec,
        )
    }

    #[tokio::test]
    async fn dispatch_ignores_non_vdfs_path() {
        let (fs, _) = fs_roots();
        let ctx = ctx_empty();
        assert!(dispatch(&fs, "chat/send", &ctx).await.is_none());
        assert!(dispatch(&fs, "", &ctx).await.is_none());
    }

    /// `vdfs/action`：动作标识与展示地址原样转发，本层不解释语义
    #[tokio::test]
    async fn action_forwards_verb_and_relative_path() {
        let (fs, rec) = fs_roots();
        let ctx = ctx_with(json!({ "path": ".vdfsv2/mem/a.txt", "action": "ping" }));
        let resp = dispatch(&fs, VDFS_ACTION, &ctx).await.unwrap().unwrap();
        let data = resp.get::<VdfsActionResult>().unwrap();
        assert_eq!(data.action, "ping");
        assert!(data.ok);
        assert_eq!(rec.seen(), vec!["action:ping"], "provider 只收到动作标识");

        // 空动作标识 → 拒绝（不打扰 provider）
        let bad = ctx_with(json!({ "path": ".vdfsv2/mem/a.txt", "action": "  " }));
        assert!(dispatch(&fs, VDFS_ACTION, &bad).await.unwrap().is_err());
    }

    /// 类别根列表：门面把内部口径回填成 `.vdfsv2/...` 展示地址 + `ext` 推导
    #[tokio::test]
    async fn list_category_root_fills_paths_and_ext() {
        let (fs, rec) = fs_roots();
        let ctx = ctx_with(json!({ "path": ".vdfsv2/mem" }));
        let resp = dispatch(&fs, VDFS_LIST, &ctx).await.unwrap().unwrap();
        let data = resp.get::<VdfsListResponse>().unwrap();
        assert_eq!(rec.seen(), vec![""], "provider 收到的是相对路径 \"\"");
        assert_eq!(data.path, ".vdfsv2/mem");
        assert_eq!(data.node.name, "mem", "目录自身节点 = 类别根");
        assert_eq!(data.items.len(), 2);
        assert_eq!(data.items[0].path, ".vdfsv2/mem/a.txt");
        assert_eq!(data.items[0].ext.as_deref(), Some("txt"));
        assert_eq!(data.items[1].path, ".vdfsv2/mem/sub");
        assert!(data.items[1].is_dir());
        assert!(data.items[1].ext.is_none());
    }

    /// 深层地址：门面在进出两处各做一次口径映射，provider 始终只见相对路径
    #[tokio::test]
    async fn deep_virtual_paths_pass_through_relative() {
        let (fs, rec) = fs_roots();
        let ctx = ctx_with(json!({ "path": ".vdfsv2/mem/sub" }));
        let resp = dispatch(&fs, VDFS_LIST, &ctx).await.unwrap().unwrap();
        let data = resp.get::<VdfsListResponse>().unwrap();
        assert_eq!(
            rec.seen(),
            vec!["sub"],
            "门面把 .vdfsv2/mem/sub 拆成相对路径 sub"
        );
        assert_eq!(data.items[0].name, "b.md");
        assert_eq!(data.items[0].path, ".vdfsv2/mem/sub/b.md");
        assert_eq!(data.node.name, "sub");
    }

    #[tokio::test]
    async fn list_unknown_dir_is_not_found_with_hint() {
        let (fs, _) = fs_roots();
        let ctx = ctx_with(json!({ "path": ".vdfsv2/nope" }));
        let err = dispatch(&fs, VDFS_LIST, &ctx).await.unwrap().unwrap_err();
        assert!(matches!(err, PluginError::NotFound(_)));
        assert!(err.to_string().contains("mem"), "提示现有子目录");
    }

    /// 路径穿越在访问层被拦截，根与 provider 永远拿到安全路径
    #[tokio::test]
    async fn traversal_path_rejected() {
        let (fs, _) = fs_roots();
        let ctx = ctx_with(json!({ "path": ".vdfsv2/mem/../../etc" }));
        let err = dispatch(&fs, VDFS_LIST, &ctx).await.unwrap().unwrap_err();
        assert!(matches!(err, PluginError::ValidationError(_)));
    }

    #[tokio::test]
    async fn stat_read_and_backfill() {
        let (fs, _) = fs_roots();

        let ctx = ctx_with(json!({ "path": ".vdfsv2/mem/a.txt" }));
        let resp = dispatch(&fs, VDFS_READ, &ctx).await.unwrap().unwrap();
        let c = resp.get::<VdfsContent>().unwrap();
        assert_eq!(c.text.as_deref(), Some("hello"));
        assert_eq!(
            c.path, ".vdfsv2/mem/a.txt",
            "provider 未填 path，由访问层回填"
        );

        let ctx = ctx_with(json!({ "path": ".vdfsv2/mem/sub/b.md" }));
        let n = dispatch(&fs, VDFS_STAT, &ctx)
            .await
            .unwrap()
            .unwrap()
            .get::<VdfsNode>()
            .unwrap();
        assert_eq!(n.path, ".vdfsv2/mem/sub/b.md");
        assert_eq!(n.effective_ext().as_deref(), Some("md"));
        assert_eq!(n.access.flags(), "r");

        // 类别根节点由组合视图合成（provider 不知道自己的类别名）
        let ctx = ctx_with(json!({ "path": ".vdfsv2/mem" }));
        let n = dispatch(&fs, VDFS_STAT, &ctx)
            .await
            .unwrap()
            .unwrap()
            .get::<VdfsNode>()
            .unwrap();
        assert_eq!(n.kind, VDFS_KIND_DIR);
        assert_eq!(n.name, "mem");
        assert_eq!(n.title, "内存子树");
    }

    /// 机器级守卫：内容缺失即拒绝，provider 不会被调用
    #[tokio::test]
    async fn write_requires_content_and_maps_validation_fields() {
        let (fs, _) = fs_roots();

        let ctx = ctx_with(json!({ "path": ".vdfsv2/mem/a.txt" }));
        assert!(matches!(
            dispatch(&fs, VDFS_WRITE, &ctx).await.unwrap(),
            Err(PluginError::ValidationError(_))
        ));

        let ctx = ctx_with(json!({ "path": ".vdfsv2/mem/a.txt", "text": "x" }));
        let w = dispatch(&fs, VDFS_WRITE, &ctx)
            .await
            .unwrap()
            .unwrap()
            .get::<VdfsWriteResponse>()
            .unwrap();
        assert_eq!(w.path, ".vdfsv2/mem/a.txt");
        assert_eq!(w.etag.as_deref(), Some("v1"));

        // 字段级校验错误：载荷序列化为 JSON 置于错误文案位，可解析还原
        let ctx = ctx_with(json!({ "path": ".vdfsv2/mem/a.txt", "text": "bad" }));
        let err = dispatch(&fs, VDFS_WRITE, &ctx).await.unwrap().unwrap_err();
        let PluginError::ValidationError(text) = err else {
            panic!("应为校验错误");
        };
        let parsed: VdfsValidationError = serde_json::from_str(&text).expect("应为结构化校验载荷");
        assert_eq!(parsed.fields[0].field, "text");
    }

    #[tokio::test]
    async fn delete_mkdir_and_dir_root_guards() {
        let (fs, _) = fs_roots();

        let ctx = ctx_with(json!({ "path": ".vdfsv2/mem/sub", "recursive": true }));
        assert!(dispatch(&fs, VDFS_DELETE, &ctx).await.unwrap().is_ok());

        let ctx = ctx_with(json!({ "path": ".vdfsv2/mem" }));
        assert!(matches!(
            dispatch(&fs, VDFS_DELETE, &ctx).await.unwrap(),
            Err(PluginError::Forbidden(_))
        ));

        let ctx = ctx_with(json!({ "path": ".vdfsv2/mem" }));
        assert!(matches!(
            dispatch(&fs, VDFS_MKDIR, &ctx).await.unwrap(),
            Err(PluginError::ValidationError(_))
        ));
    }

    #[tokio::test]
    async fn dir_root_is_not_readable() {
        let (fs, _) = fs_roots();
        let ctx = ctx_with(json!({ "path": ".vdfsv2/mem" }));
        let err = dispatch(&fs, VDFS_READ, &ctx).await.unwrap().unwrap_err();
        assert!(matches!(err, PluginError::Forbidden(_)));
    }

    /// 同类别内移动：展示地址进、相对路径到 provider
    #[tokio::test]
    async fn same_category_move_is_forwarded() {
        let (fs, _) = fs_roots();
        let ctx = ctx_with(json!({ "from": ".vdfsv2/mem/a.txt", "to": ".vdfsv2/mem/b.txt" }));
        let resp = dispatch(&fs, VDFS_MOVE, &ctx).await.unwrap().unwrap();
        let m = resp.get::<VdfsMoveResponse>().unwrap();
        assert_eq!(m.from, ".vdfsv2/mem/a.txt");
        assert_eq!(m.to, ".vdfsv2/mem/b.txt");
    }

    /// 跨子目录移动由组合根拒绝（门面只拦「两半之间」，子目录间归虚拟层自持）
    #[tokio::test]
    async fn cross_category_move_is_rejected() {
        let rec = Rec::new();
        let a: DynVdfsProvider = rec.clone();
        let b: DynVdfsProvider = Arc::new(Bare);
        let fs: DynVdfsProvider = Arc::new(UnifiedFs::with_physical(
            Arc::new(TestRoot {
                dirs: vec![("mem", a), ("bare", b)],
            }),
            Arc::new(PhysicalFs::new()),
        ));
        let ctx = ctx_with(json!({ "from": ".vdfsv2/mem/a.txt", "to": ".vdfsv2/bare/a.txt" }));
        assert!(matches!(
            dispatch(&fs, VDFS_MOVE, &ctx).await.unwrap(),
            Err(PluginError::ValidationError(_))
        ));
    }

    /// 系统资源与磁盘文件之间不可移动（门面判定，先于触达任何一层）
    #[tokio::test]
    async fn cross_half_move_is_rejected() {
        let (fs, rec) = fs_roots();
        let ctx = ctx_with(json!({ "from": ".vdfsv2/mem/a.txt", "to": "b.txt" }));
        assert!(matches!(
            dispatch(&fs, VDFS_MOVE, &ctx).await.unwrap(),
            Err(PluginError::ValidationError(_))
        ));
        assert!(rec.seen().is_empty(), "判定发生在触达 provider 之前");
    }

    #[tokio::test]
    async fn watch_unwatch_forward_relative_path() {
        let (fs, rec) = fs_roots();
        let ctx = ctx_with(json!({ "path": ".vdfsv2/mem/sub" }));
        assert!(dispatch(&fs, VDFS_WATCH, &ctx).await.unwrap().is_ok());
        assert!(dispatch(&fs, VDFS_UNWATCH, &ctx).await.unwrap().is_ok());
        assert_eq!(rec.seen(), vec!["sub", "sub"], "provider 收到相对路径");
    }

    /// 树遍历从虚拟根均匀展开：类别本身也是树的一层
    #[tokio::test]
    async fn tree_walks_uniformly_from_root() {
        let (fs, _) = fs_roots();
        let ctx = ctx_with(json!({ "path": ".vdfsv2" }));
        let t = dispatch(&fs, VDFS_TREE, &ctx)
            .await
            .unwrap()
            .unwrap()
            .get::<VdfsTreeResponse>()
            .unwrap();
        let paths: Vec<&str> = t.nodes.iter().map(|n| n.path.as_str()).collect();
        assert_eq!(
            paths,
            vec![
                ".vdfsv2/mem",
                ".vdfsv2/mem/a.txt",
                ".vdfsv2/mem/sub",
                ".vdfsv2/mem/sub/b.md"
            ]
        );
        assert!(!t.truncated);
    }

    #[tokio::test]
    async fn tree_respects_depth_and_limit() {
        let (fs, _) = fs_roots();

        let ctx = ctx_with(json!({ "path": ".vdfsv2/mem", "depth": 1 }));
        let t = dispatch(&fs, VDFS_TREE, &ctx)
            .await
            .unwrap()
            .unwrap()
            .get::<VdfsTreeResponse>()
            .unwrap();
        assert_eq!(
            t.nodes.iter().map(|n| n.path.as_str()).collect::<Vec<_>>(),
            vec![".vdfsv2/mem/a.txt", ".vdfsv2/mem/sub"],
            "depth=1 → 只到直接子节点"
        );

        let ctx = ctx_with(json!({ "path": ".vdfsv2", "limit": 2 }));
        let t = dispatch(&fs, VDFS_TREE, &ctx)
            .await
            .unwrap()
            .unwrap()
            .get::<VdfsTreeResponse>()
            .unwrap();
        assert!(t.truncated);
        assert_eq!(t.nodes.len(), 2);
    }

    /// 物理半经统一分发读写磁盘：WORKDIR 由调用级参数透传到物理层
    #[tokio::test]
    async fn physical_half_reads_and_writes_disk() {
        let dir = std::env::temp_dir().join(format!(
            "symbio-host-{}-{:?}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();

        let fs: DynVdfsProvider = Arc::new(UnifiedFs::new(empty_root()));
        let mk_ctx = |payload: Value| {
            let c: Arc<dyn InvokeRequest> = Arc::new(SimpleRequest::new(None, None));
            c.set(WORKDIR, dir.to_string_lossy().into_owned());
            c.set_payload(payload).unwrap();
            c
        };

        let wctx = mk_ctx(json!({ "path": "hello.txt", "text": "hi" }));
        let w = dispatch_with(&fs, VDFS_WRITE, &wctx, call_params(&wctx))
            .await
            .unwrap()
            .unwrap()
            .get::<VdfsWriteResponse>()
            .unwrap();
        assert!(w.created);
        assert!(dir.join("hello.txt").exists());

        let rctx = mk_ctx(json!({ "path": "hello.txt" }));
        let c = dispatch_with(&fs, VDFS_READ, &rctx, call_params(&rctx))
            .await
            .unwrap()
            .unwrap()
            .get::<VdfsContent>()
            .unwrap();
        assert_eq!(c.text.as_deref(), Some("hi"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 变更事件：门面已把路径补成展示地址，本层**原样透传**（不再换信封）
    #[test]
    fn change_event_passes_display_paths_through() {
        // 物理半的地址原样保留
        let n = VdfsChange::bare("README.md");
        assert_eq!(n.path, "README.md");
        assert!(n.data.is_none());
    }

    // ==================== 根解析 ====================

    /// 无能力管理器、无父插件 → 统一文件系统仍可用：虚拟层为空、物理层照常
    #[tokio::test]
    async fn resolve_fs_degrades_to_empty_vfs() {
        let fs = resolve_fs(None, &ctx_empty()).await;

        let ctx = ctx_with(json!({ "path": ".vdfsv2" }));
        let data = dispatch(&fs, VDFS_LIST, &ctx)
            .await
            .unwrap()
            .unwrap()
            .get::<VdfsListResponse>()
            .unwrap();
        assert_eq!(data.path, ".vdfsv2");
        assert!(data.items.is_empty(), "虚拟层降级为空目录而非报错");

        // 具体虚拟地址一律 NotFound
        let ctx = ctx_with(json!({ "path": ".vdfsv2/mem" }));
        assert!(matches!(
            dispatch(&fs, VDFS_LIST, &ctx).await.unwrap(),
            Err(PluginError::NotFound(_))
        ));

        // 物理半不受降级影响（缺 WORKDIR 是接线错误，不是 NotFound）
        let ctx = ctx_with(json!({ "path": "" }));
        assert!(matches!(
            dispatch(&fs, VDFS_LIST, &ctx).await.unwrap(),
            Err(PluginError::InternalError(_))
        ));
    }

    /// `ctx` 已带能力管理器 → 直接读其中的根（LLM 链路）
    #[tokio::test]
    async fn resolve_fs_prefers_visitor_slot() {
        let rec = Rec::new();
        let visitor: Arc<dyn CapabilityVisitor> = Arc::new(DefaultToolVisitor::new());
        visitor.register_vdfs_root(root_with(&rec)).await;

        let ctx: Arc<dyn InvokeRequest> = Arc::new(SimpleRequest::new(None, None));
        ctx.set(CAPABILITY_VISITOR, visitor);

        let fs = resolve_fs(None, &ctx).await;
        let data = dispatch(&fs, VDFS_LIST, &ctx_with(json!({ "path": ".vdfsv2/mem" })))
            .await
            .unwrap()
            .unwrap()
            .get::<VdfsListResponse>()
            .unwrap();
        assert_eq!(data.node.name, "mem");
        assert_eq!(data.items.len(), 2);
    }

    /// 无能力管理器 → 经 `Plugin::get_vfs_provider` 直接取父插件的虚拟层根（前端链路）
    #[tokio::test]
    async fn resolve_fs_fetches_root_via_trait_method() {
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
                _ctx: Arc<dyn InvokeRequest>,
            ) -> InvokeResponse<PluginPayload> {
                Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
            }

            /// 系统链路：直接暴露自己的虚拟层根（与 LLM 链路经 `register_vdfs_root` 收集互不干扰）
            fn get_vfs_provider(self: Arc<Self>) -> Option<Arc<dyn VdfsProvider>> {
                Some(root_with(&Rec::new()))
            }
        }

        let parent: Arc<dyn Plugin> = Arc::new(FakeContainer);
        let fs = resolve_fs(Some(&parent), &ctx_empty()).await;

        let items = dispatch(&fs, VDFS_LIST, &ctx_with(json!({ "path": ".vdfsv2" })))
            .await
            .unwrap()
            .unwrap()
            .get::<VdfsListResponse>()
            .unwrap()
            .items;
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].name, "mem");
        assert_eq!(items[0].path, ".vdfsv2/mem");
    }
}
