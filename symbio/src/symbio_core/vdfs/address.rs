//! 相对地址 ↔ 绝对地址：**当前父地址**与它的唯一来源
//!
//! ## 相对地址是常态，绝对地址是例外
//!
//! [`VdfsProvider`](crate::symbio_core::vdfs_provider::VdfsProvider) 收到的地址
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

use crate::symbio_core::{InvokeRequest, InvokeRequestExt, VDFS_PARENT_ADDR};

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
pub(crate) fn absolute_addr(ctx: &Arc<dyn InvokeRequest>, rel: &str) -> String {
    join_addr(ctx.get(VDFS_PARENT_ADDR).unwrap_or_default().as_str(), rel)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 拼接规则：单级 / 多级 / 两边多余分隔符 / 无父可接
    #[test]
    fn join_addr_normalizes_both_sides() {
        assert_eq!(join_addr("@root", ""), "@root");
        assert_eq!(join_addr("@root/", "/"), "@root");
        assert_eq!(
            join_addr("@root/session", "abc/AGENTS.md"),
            "@root/session/abc/AGENTS.md"
        );
        assert_eq!(join_addr("@root/session/", "/abc/"), "@root/session/abc");
        assert_eq!(join_addr("", "session/x"), "session/x");
        assert_eq!(join_addr("  ", "session/x"), "session/x");
    }

    /// 静态声明可达（本 crate 编译进了 vdfs 插件，它提交了根名）。
    /// 具体的名字归那个插件——这里只钉「声明机制在工作」。
    #[test]
    fn declaration_is_reachable_and_normalized() {
        let root = declared_addr_root().expect("vdfs 插件应已静态声明根名");
        assert!(!root.is_empty());
        assert!(
            !root.ends_with('/') && !root.starts_with('/'),
            "应已归一化: {root}"
        );
    }

    /// 转发跳的改写规则：顶层落到声明的根；嵌套时在当前父地址上续接
    #[test]
    fn descend_starts_at_root_then_layers() {
        let root = declared_addr_root().unwrap();
        // 顶层（转发方自己没有父地址）→ 根 + 本级名
        assert_eq!(descend_addr("", "session"), format!("{root}/session"));
        assert_eq!(descend_addr("  ", "session"), format!("{root}/session"));
        // 嵌套（转发方已有父地址）→ 原地续接，不回落到根
        assert_eq!(
            descend_addr("@elsewhere/agent/b1", "sub"),
            "@elsewhere/agent/b1/sub"
        );
    }
}
