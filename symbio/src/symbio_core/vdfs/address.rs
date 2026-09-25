//! 相对地址 ↔ 绝对地址：**当前父地址**与它的唯一来源
//!
//! ## 相对地址是常态，绝对地址是例外
//!
//! [`VdfsProvider`](crate::symbio_core::vdfs::VdfsProvider) 收到的地址
//! 一律是**自身子树内的相对地址**（`""` = 自身根）——绝大多数访问只跟相对地址
//! 打交道，从不需要知道自己在地址空间里的绝对位置。只有少数**协议级**场合需要
//! 全局地址：提示词里印给模型的可编辑地址、错误信息里给用户指路等。
//! 本模块只服务这些场合。
//!
//! ## 绝对地址怎么来：一层层往下传，不写死、不问全局
//!
//! ```text
//! plugins/vdfs  ──静态声明根名──▶ AddrRootDecl（编译期随二进制生效）
//! composite     ──装配子插件时──▶ 当前父地址 = join(根名, 子插件名)，经 ctx 传递
//! 子插件        ──存下父地址────▶ 绝对地址 = join(父地址, 相对地址)，本地纯数据操作
//! ```
//!
//! - 根名的字面量**只在 vdfs 插件内**（`plugins/vdfs/fs.rs`），它经
//!   [`AddrRootDecl`] 静态声明——早于任何插件实例构造，容器转发时即可读取；
//! - 当前父地址是**上下文数据**（`VDFS_PARENT_ADDR`）：父插件把请求转发给子
//!   插件时改写它，与 `WORKDIR` / `SESSION_ID` 同类；
//! - 拼接规则全项目只有一份：[`join_addr`]；带上下文的封装入口是
//!   [`absolute_addr`]。
//!
//! ## 没有声明时怎么办
//!
//! [`declared_addr_root`] 返回 `None`（构建里没有 vdfs 插件），容器传给子插件
//! 的父地址退化为裸槽名，绝对地址随之退化为根相对地址——可诊断的降级，
//! 不是错误：整个虚拟资源面本就不存在。

use std::sync::Arc;

use crate::symbio_core::{PluginInvokeRequest, PluginInvokeRequestExt, VDFS_PARENT_ADDR};

/// 地址空间根的**静态声明** —— 由 vdfs 插件提交（`inventory::submit!`）。
///
/// 根名是 vdfs 插件的挂载规则，字面量只存在于那个插件里；静态声明使它在
/// **任何插件实例构造之前**就可用（容器装配子插件时要用），同时不引入
/// 「容器认识 vdfs」的依赖——容器只认本类型，不认识任何具体插件。
pub struct AddrRootDecl(pub &'static str);

inventory::collect!(AddrRootDecl);

/// 已声明的地址空间根名；构建里没有 vdfs 插件时为 `None`。
///
/// 归一化（去首尾空白与分隔符）在此一次完成：声明方给带尾部 `/` 的形态
/// 与裸名都是同一个根。本模块（core）对具体名字零知识——它由 vdfs 插件
/// 声明，字面量只存在于那个插件里。
pub(crate) fn declared_addr_root() -> Option<&'static str> {
    inventory::iter::<AddrRootDecl>()
        .next()
        .map(|d| d.0.trim().trim_matches('/'))
}

/// 纯核：`join_addr(parent, rel)` —— 把相对地址接到父地址上，拼出绝对地址。
///
/// - `join_addr("@root/session", "abc/AGENTS.md")` → `@root/session/abc/AGENTS.md`
/// - `join_addr("session", "x")` → `session/x`（父地址本身也可以是相对的）
/// - `join_addr("", "x")` → `x`（无父可接：原样返回相对地址，见模块文档的降级说明）
pub(crate) fn join_addr(parent: &str, rel: &str) -> String {
    let parent = parent.trim().trim_matches('/');
    let rel = rel.trim().trim_matches('/');
    match (parent.is_empty(), rel.is_empty()) {
        (true, _) => rel.to_string(),
        (false, true) => parent.to_string(),
        (false, false) => format!("{parent}/{rel}"),
    }
}

/// **转发跳的改写规则**：把当前父地址往下一级。
///
/// `current` 是转发方自己拿到的当前父地址——嵌套转发时已有值（往它下面接）；
/// 顶层为空，落到静态声明的根。`name` 是本级挂载名（子插件名 / 目录名）。
/// 规则全项目只有一份：容器的请求转发、能力收集与 vdfs 协议派发共用。
pub(crate) fn descend_addr(current: &str, name: &str) -> String {
    let base = if current.trim().is_empty() {
        declared_addr_root().unwrap_or("")
    } else {
        current
    };
    join_addr(base, name)
}

/// **插件侧的封装入口**：上下文父地址 + 相对地址 → 绝对地址。
///
/// 只在少数协议级场合调用（提示词里印给模型的可编辑地址、错误信息指路等）；
/// 上下文没有父地址（顶层请求 / 无 vdfs 装配）时退化为根相对地址。
pub(crate) fn absolute_addr(ctx: &Arc<dyn PluginInvokeRequest>, rel: &str) -> String {
    join_addr(ctx.get(VDFS_PARENT_ADDR).unwrap_or_default().as_str(), rel)
}

// ==================== 路径判定（地址契约的唯一实现） ====================
//
// 下面三个函数是**地址规则**的实现，因此归本模块所有：任何按路径段比较、
// 或需要收敛坐标系的场合都调用它们，不得各写一份。
//
// 历史教训：`..` 判定曾按「是否以 `../` 开头」实现，Windows 下
// `src\..\..\..\Windows` 既不以 `../` 也不以 `..\` 开头，直接绕过守卫；
// 黑名单前缀曾用裸 `starts_with("/etc")`，把 `/etcfoo` 一并误伤。
// 两处 bug 同源——**按字符串前缀代替按路径段比较**。

/// 路径中是否含 `..` 段——**两种分隔符都算**。
///
/// 只查 `/` 会让 Windows 的 `src\..\..\..\Windows` 绕过守卫；只查带分隔符的
/// `../` / `..\` 前缀会放过 `a/..`（`..` 收尾）。因此按**段**判定，与分隔符无关。
pub fn has_parent_segment(path: &str) -> bool {
    path.split(['/', '\\']).any(|seg| seg == "..")
}

/// `path` 是否落在 `prefix` 之内——相等，或紧随一个分隔符。
///
/// 前缀必须按**路径段**比较：裸 `starts_with("/etc")` 会把 `/etcfoo` 误伤。
pub fn path_within(path: &str, prefix: &str) -> bool {
    let prefix = prefix.trim_end_matches(['/', '\\']);
    if prefix.is_empty() {
        return false;
    }
    match path.strip_prefix(prefix) {
        Some(rest) => rest.is_empty() || rest.starts_with('/') || rest.starts_with('\\'),
        None => false,
    }
}

#[cfg(test)]
#[path = "address.test.rs"]
mod tests;
