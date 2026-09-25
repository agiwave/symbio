//! 核心 VdfsProvider —— 统一资源访问的唯一契约（唯一接口：`dispatch`）。
//!
//! 本模块是 VDFS 的 **centerpiece**，只暴露**纯接口**：
//! - [`VdfsProvider`] 只有一个方法 [`VdfsProvider::dispatch`]：
//!   `dispatch(ctx, path, req)` —— **`path` 是第一个分发键**（先按地址找到资源域，
//!   再由域内实现决定操作怎么落地），`req`（[`VdfsRequest`] 枚举）只携带**操作的
//!   载荷**（写什么、删不递归、动作是什么……）；列 / 读 / 写 / 删 / 建 / 动作 /
//!   订阅全部落在枚举变体上；
//! - 每个资源域 = 一份 [`VdfsProvider`] 实现，`Arc<dyn VdfsProvider>` 是使用方与
//!   实现方之间**唯一**的交换物；
//! - 线上形状（`vdfs/*` 请求 / 响应信封、协议路径常量）定义在 vdfs 插件内部
//!   （`plugins/vdfs/protocol.rs`），**core 不暴露这些类型**。
//!
//! **分层、依赖方向、开放边界（本模块可原样抽出为独立 crate）见
//! `docs/design/vdfs.md` §2**——此处不复述。
//!
//! 数据模型速览：
//!
//! - 一切资源 = 目录树上的**节点**（[`VdfsNode`]），地址 = 树内相对路径
//!   `<目录>/<rel>`（全路径由使用方拼接、回填；provider 不知道自己被放在哪层目录下）；
//! - 节点的能力 = 四个**访问位**（[`VdfsAccess`]：`r` 读 / `w` 写 / `l` 列 / `t` 遍历）；
//! - 内容 = [`VdfsContent`]（文本 `text` 或二进制 `b64`，互斥）；
//! - 呈现 = 节点的 `ext`（扩展名）→ 使用方选渲染器；渲染器所需描述经 `schema` 透传；
//! - 变更 = [`VdfsChange`]（子树内**相对路径** + 可选**业务载荷** `data`，
//!   缺失 = 回读收敛），经 [`VdfsChangeSink`] 由使用方补成展示地址后投递。

use async_trait::async_trait;
use std::sync::Arc;

use super::context::VdfsContext;
use super::error::VdfsResult;
use super::request::{VdfsRequest, VdfsResponse};

// ==================== provider trait ====================

/// VDFS provider —— 把一个资源域暴露为一棵可被使用的资源子树。
///
/// **provider 不知道自己被挂在哪里**：挂载名由使用方在注册时选定，trait 上没有
/// 任何与挂载相关的成员；自述（标题 / 描述 / 顺序 / 图标 / 根访问位）由
/// [`crate::symbio_core::PluginMeta`] 承载（`Plugin::meta()`）。
///
/// ## 唯一接口
///
/// [`Self::dispatch`] 收 `(ctx, path, req)`：**`path` 是第一个分发键**——实现方
/// 先按它定位资源（转发型实现剥首段找下一层；叶子实现按段找自己的资源），再由
/// `req` 决定操作怎么落地。实现方按变体 `match`，**只实现自己支持的操作**——
/// 其余臂返回 [`VdfsError::NotImplemented`]，使用方据此隐藏对应入口。
///
/// ## 自述一律走 `dispatch`，trait 上不再有第二条通道
///
/// 「根的自述」里有一项**进不了** `PluginMeta`：[`VdfsNode::new_type`] 的
/// `schema` 可能要运行期汇流（如 session 的选项定义来自一次 options 广播），
/// 而 `PluginMeta` 是同步纯数据。
///
/// 它的出口不是 trait 上的另一个方法，而是**节点自述本身**：容器合成挂载点节点
/// 时，向该 provider 发一次 `dispatch(ctx, "", Stat)`——**provider 对空路径的
/// `Stat` 就是它对自己根的描述**（这一条早就成立：各 provider 的根 `Stat` 都
/// 返回「名字留空、由使用方回填」的根节点）。容器只从那份描述里取 `new_type`，
/// 其余字段仍以 `PluginMeta` 为准。
///
/// 这样做的理由不是「少一个方法」，而是**通道只有一条才不会有第二种答案**：
/// 根与更深层的节点（由 provider 自己在 `list` 里给出 [`VdfsNode::new_type`]）
/// 走的是同一条路，使用方不必知道「这个节点是不是根」才能问它「你能新建什么」。
/// 详见 `docs/DECISIONS.md` ADR-030。
///
/// ## 实现约定
///
/// - **地址是本子树内的相对路径**（`""` = 自身根），已规范化、无穿越风险；
/// - **`access` 是能力声明**：使用方与消费者只看访问位，不做类型特判；
/// - **校验归实现方**：写入的必填 / 范围 / 格式校验在实现内完成（见
///   [`VdfsRequest`] 的语义一节），失败返回 [`VdfsError::Invalid`]（可带字段级
///   错误）；
/// - **线程安全**：`&self` 可能被并发调用。
#[async_trait]
pub trait VdfsProvider: Send + Sync + 'static {
    /// 唯一入口：先按 `path` 定位资源域，再按 `req` 变体执行操作
    /// （各操作语义见 [`VdfsRequest`] 的模块级文档）
    async fn dispatch(
        &self,
        ctx: &VdfsContext,
        path: &str,
        req: VdfsRequest,
    ) -> VdfsResult<VdfsResponse>;
}

/// 类型别名：便于使用方在容器里存放 `dyn VdfsProvider`
pub type DynVdfsProvider = Arc<dyn VdfsProvider>;
