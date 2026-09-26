//! 统一地址空间门面 —— 一棵树的两半按前缀分流
//!
//! ## 唯一的地址规则
//!
//! ```text
//! <根>              根目录（系统资源清单）           ─┐
//! <根>/<子目录>/…    某插件子树                       ├─▶ 虚拟层（容器注册的根）
//!                                                   ─┘
//! README.md        工作目录相对地址                 ─┐
//! src/main.rs      同上                              ├─▶ 物理层（磁盘真实文件）
//! D:/tmp/a.txt     绝对路径                          ─┘
//! ```
//!
//! **判别只有一条**：规范化后的地址是否以根名（[`VDFS_ADDR_ROOT`]）打头。
//! 是 → 虚拟；否 → 物理。
//! 前端协议、LLM 工具两条链路共用这一个 [`UnifiedFs`]，因此翻译只存在于本文件一处，
//! 页面与工具都不需要知道展示口径与树内口径的差异。
//!
//! ## 根名归本插件，且只归本插件
//!
//! 根名是**本插件的挂载规则**：它就是这么挂在系统资源上的。字面量因此只允许出现在
//! 本插件内（[`VDFS_ADDR_ROOT`]，并经 `AddrRootDecl` 静态声明给 core 收集），
//! 对外只经两个口子出去，别的模块都不必知道它：
//!
//! | 消费者 | 拿到的形态 |
//! |---|---|
//! | 其它插件要印一个可编辑地址（提示词里的「地址：…」） | **上下文父地址 + 相对地址**（`absolute_addr`）：转发时由容器写入，插件不持有根名 |
//! | 前端要进入地址空间 | `vdfs/root`（[`crate::symbio_core::ROUTE_VDFS_ROOT`]）：**不给地址**就能列出根，回包里的 `path` 即根地址 |
//!
//! 「改挂载名」因此只需改本文件这一行。
//!
//! ## 为什么需要「口径」转换
//!
//! 虚拟层内部（容器注册的根及其下各 provider）是通用机制，provider 只认
//! **自身子树内的相对路径**（core 不该知道宿主把根目录叫做什么）。所以本门面
//! 在进出两处各做一次轻量映射：
//!
//! | 方向 | 映射 |
//! |---|---|
//! | 请求进入虚拟层 | `<根>` → `""`；`<根>/session/x` → `session/x` |
//! | 结果回到消费者 | `session/x` → `<根>/session/x` |
//!
//! 物理层不需要映射：它的地址本来就是根名之外的样子。
//!
//! ## 本层不认识目录清单
//!
//! 根目录下有多少子目录、叫什么，由容器与注册决定；本文件只做前缀分流，
//! 新增一类资源不需要改动这里任何一行。

use super::physical::PhysicalFs;
use crate::symbio_core::{
    vdfs_has_parent_segment, DynVdfsProvider, VdfsChange, VdfsChangeSink, VdfsContext, VdfsError,
    VdfsItem, VdfsProvider, VdfsResult,
};
use crate::symbio_core::{VdfsRequest, VdfsResponse};
use async_trait::async_trait;
use std::sync::Arc;

/// 虚拟根目录的字面名字：系统资源统一在此目录之下。
///
/// **全项目唯一一处根名字面量**——它就是本插件的挂载规则，因此只允许写在这里。
/// 改这个值（如改成 `.vdfsv22`）即可整体换名，其余代码不持有它：
///
/// - 其它插件要印可编辑地址 → 用**上下文父地址**拼相对地址（core 的
///   `absolute_addr`，转发时容器已写入父地址），本插件之外无人认识这个名字；
/// - 前端要进入地址空间 → `vdfs/root`（[`crate::symbio_core::ROUTE_VDFS_ROOT`]），
///   回包里的 `path` 即根地址，前端把它当**运行期数据**持有。
///
/// 根名同时经 `AddrRootDecl` 静态声明（编译期随二进制生效，容器装配子插件
/// 前即可读取）。
pub const VDFS_ADDR_ROOT: &str = ".vdfsv2";

/// 规范化后的地址落在哪一半
#[derive(Debug, Clone, PartialEq, Eq)]
enum Half {
    /// 虚拟层：树内相对路径（`""` = 根目录）
    Virtual(String),
    /// 物理层：工作目录相对地址（`""` = 工作目录根）
    Physical(String),
}

/// 规范化地址：折叠空段与 `.`、拒绝 `..` 穿越、去掉首部分隔符。
///
/// **不强加前导 `/`**——虚拟地址以 `.vdfsv2` 打头、物理相对地址以名字打头、
/// 绝对物理地址以盘符打头，三者形态各异，统一成「以 `/` 分隔的段序列」即可。
///
/// - `""` / `"."` / `"/"` → `""`（工作目录根）
/// - `".vdfsv2/session/"` → `".vdfsv2/session"`
/// - `"/src/main.rs"` → `"src/main.rs"`（首段分隔符只是分隔，不是文件系统根）
/// - 含 `..` 段 → [`VdfsError::Invalid`]
pub fn normalize_addr(raw: &str) -> VdfsResult<String> {
    // 第一道：按**段**判 `..`，且两种分隔符都算。
    //
    // 只按 `/` 分段会被 Windows 形式的 `demo/..\..\escaped` 绕过——`\` 同样是
    // 路径分隔符，下游 `Path::join` 会照着它解析。规则本体在机制层
    // [`vdfs_has_parent_segment`]，此处**复用而非再写一份**：本文件开头的地址规则
    // 说明早已定下「分段比较」，历史 bug 正是「按字符串前缀 / 单一分隔符代替
    // 按路径段比较」。
    if vdfs_has_parent_segment(raw) {
        return Err(VdfsError::invalid(format!(
            "VDFS 地址不允许向上穿越：{raw}"
        )));
    }
    let mut segs: Vec<&str> = Vec::new();
    for seg in raw.trim().split('/') {
        let seg = seg.trim();
        if seg.is_empty() || seg == "." {
            continue;
        }
        // 第二道：覆盖被空白包裹的 `..`（`" .. "`）——逐段 `trim()` 才暴露它，
        // 上面的 `vdfs_has_parent_segment` 按原样分段看不到。
        if seg == ".." {
            return Err(VdfsError::invalid(format!(
                "VDFS 地址不允许向上穿越：{raw}"
            )));
        }
        segs.push(seg);
    }
    Ok(segs.join("/"))
}

/// 规范化地址 → 那一半
///
/// `.vdfsv2` 必须**独占首段**：`.vdfsv2foo` 不算虚拟地址（按物理地址处理）。
fn half_of(addr: &str) -> Half {
    match addr.strip_prefix(VDFS_ADDR_ROOT) {
        Some("") => Half::Virtual(String::new()),
        Some(rest) if rest.starts_with('/') => {
            // 虚拟层树内口径：剥掉 `.vdfsv2/` 前缀，剩余部分原样透传
            Half::Virtual(rest[1..].to_string())
        }
        _ => Half::Physical(addr.to_string()),
    }
}

/// 虚拟层树内口径 → 对外展示地址（`session/x` → `<根>/session/x`）
///
/// 拼接规则复用机制层的那一份（`join_addr`），本插件只提供根名——「根 + 相对」
/// 的实现于是只有一处，本门面与其它插件的展示地址不会有第二种拼法。
pub fn to_display(root_rel: &str) -> String {
    crate::symbio_core::join_addr(VDFS_ADDR_ROOT, root_rel)
}

/// 解析地址并路由（所有操作的共同第一步）
fn route(raw: &str) -> VdfsResult<Half> {
    Ok(half_of(&normalize_addr(raw)?))
}

/// 统一文件系统：虚拟根 + 物理磁盘，对外是一张脸
pub struct UnifiedFs {
    /// 虚拟层根（容器注册的 root 级 provider，树内相对路径）
    virtual_root: DynVdfsProvider,
    /// 物理层（磁盘真实文件）
    physical: DynVdfsProvider,
}

impl UnifiedFs {
    /// 以容器注册的虚拟根构造，物理层用默认的 [`PhysicalFs`]
    pub fn new(virtual_root: DynVdfsProvider) -> Self {
        Self::with_physical(virtual_root, Arc::new(PhysicalFs::new()))
    }

    /// 显式注入物理层（测试可换成内存实现）
    pub fn with_physical(virtual_root: DynVdfsProvider, physical: DynVdfsProvider) -> Self {
        Self {
            virtual_root,
            physical,
        }
    }

    /// 树内口径 → 展示口径（`session/x` → `.vdfsv2/session/x`）。
    ///
    /// **只改非空 `path`**：空 `path` 是「provider 未填」的信号，由访问层
    /// （`host::fill_paths`）按请求地址回填成 `<父地址>/<子名>`；若在这里把它
    /// 补成 `.vdfsv2`，信号即被破坏，子节点会被错认成根。
    ///
    /// 地址落在**条目**上（[`VdfsItem`]），节点本身是纯自述——本函数因此只碰
    /// `it.path`。
    fn retag(&self, items: &mut [VdfsItem]) {
        for it in items.iter_mut() {
            if !it.path.is_empty() {
                it.path = to_display(&it.path);
            }
        }
    }
}

#[async_trait]
impl VdfsProvider for UnifiedFs {
    /// 唯一入口：按 `path` 前缀分流（虚拟 / 物理），再把请求整体递下去。
    ///
    /// 请求载荷里没有地址字段（见 `VdfsRequest` 的「没有 `Move`」一节），所以
    /// 本层只需翻译 `path` 这一个地址。
    async fn dispatch(
        &self,
        ctx: &VdfsContext,
        path: &str,
        req: VdfsRequest,
    ) -> VdfsResult<VdfsResponse> {
        match route(path)? {
            Half::Virtual(v) => self.dispatch_virtual(ctx, &v, req).await,
            Half::Physical(p) => self.physical.dispatch(ctx, &p, req).await,
        }
    }
}

impl UnifiedFs {
    /// 虚拟半的派发：树内相对路径递给虚拟根，结果的**条目地址**翻回展示口径。
    ///
    /// **只翻译已填的**：空 `path` 是「provider 未填」的信号，由访问层
    /// （`host::fill_paths`）按请求地址回填；若在这里把它补成 `.vdfsv2`，
    /// 信号即被破坏，子节点会被错认成根。
    async fn dispatch_virtual(
        &self,
        ctx: &VdfsContext,
        rel: &str,
        req: VdfsRequest,
    ) -> VdfsResult<VdfsResponse> {
        let root = &self.virtual_root;
        match req {
            VdfsRequest::List { .. } => {
                let mut items = root
                    .dispatch(
                        ctx,
                        rel,
                        VdfsRequest::List {
                            limit: None,
                            before: None,
                        },
                    )
                    .await?
                    .into_list()
                    .ok_or_else(|| VdfsError::internal("响应类型不匹配"))?;
                self.retag(&mut items);
                Ok(VdfsResponse::List(items))
            }
            VdfsRequest::Stat => {
                let n = root
                    .dispatch(ctx, rel, VdfsRequest::Stat)
                    .await?
                    .into_stat()
                    .ok_or_else(|| VdfsError::internal("响应类型不匹配"))?;
                Ok(VdfsResponse::Stat(n))
            }
            VdfsRequest::Read => {
                let c = root
                    .dispatch(ctx, rel, VdfsRequest::Read)
                    .await?
                    .into_read()
                    .ok_or_else(|| VdfsError::internal("响应类型不匹配"))?;
                Ok(VdfsResponse::Read(c))
            }
            VdfsRequest::Write { content } => {
                let r = root
                    .dispatch(ctx, rel, VdfsRequest::Write { content })
                    .await?
                    .into_write()
                    .ok_or_else(|| VdfsError::internal("响应类型不匹配"))?;
                Ok(VdfsResponse::Write(r))
            }
            VdfsRequest::Delete { recursive } => {
                root.dispatch(ctx, rel, VdfsRequest::Delete { recursive })
                    .await?;
                Ok(VdfsResponse::Unit)
            }
            VdfsRequest::Mkdir => {
                root.dispatch(ctx, rel, VdfsRequest::Mkdir).await?;
                Ok(VdfsResponse::Unit)
            }
            VdfsRequest::Action { action, payload } => {
                root.dispatch(ctx, rel, VdfsRequest::Action { action, payload })
                    .await
            }
            VdfsRequest::Watch { sink } => {
                // 只有虚拟层的资源会自发变更。事件路径补成展示地址——订阅者看到的
                // 坐标系与请求时一致。`map_paths` 一次覆盖全部路径（含载荷内的），
                // 不逐字段重建。
                let wrapped: VdfsChangeSink =
                    Arc::new(move |c: VdfsChange| sink(c.map_paths(to_display)));
                root.dispatch(ctx, rel, VdfsRequest::Watch { sink: wrapped })
                    .await?;
                Ok(VdfsResponse::Unit)
            }
            VdfsRequest::Unwatch => {
                root.dispatch(ctx, rel, VdfsRequest::Unwatch).await?;
                Ok(VdfsResponse::Unit)
            }
        }
    }
}

#[cfg(test)]
#[path = "fs.test.rs"]
mod tests;
