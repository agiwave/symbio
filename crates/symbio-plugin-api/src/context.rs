//! 上下文端口 —— 每轮上下文由能力包贡献，宿主只按序拼接（DE-025 D-208 / D-215）。
//!
//! ## 它补的是哪一格
//!
//! 「这一轮给模型看哪几段额外文本」过去是宿主的固定拼装：`chat_loop::inputs` 里写死
//! 三段（长期记忆 / 就绪任务 / 委派者真源），`run_recall` 把前两段的正文算出来寄存进
//! 本轮状态。每加一类上下文（RAG / 日历 / 项目文档）就要改 `loop` 一次——那正是
//! 「加能力要改架构」的残余。
//!
//! 本模块把「有哪些段」挪回能力包：
//!
//! ```text
//! 能力插件 ──submit_context_provider!──▶ inventory（编译期随二进制生效）
//! 控制面   ──ContextRequest（本轮读侧的值）──▶ 收集
//! 宿主     ──按 owner 序拼接──▶ 请求前缀
//! ```
//!
//! ## 判断与贡献的分界（D-208）
//!
//! provider **只产文本**：它不写事实、不改本轮状态、也不参与「欠什么 / 何时停」的判断。
//! 需要判断的那一段，判断仍在控制面——控制面把**判完的值**放进 [`ContextRequest`]
//! （例如已被技能路由过滤过的召回视图 [`RecallFacts`]），provider 只负责渲染。
//! 于是「换一个渲染者」不改变判断，「加一个来源」不改变控制面。
//!
//! ## 入参为什么是一包值
//!
//! [`ContextRequest`] 里没有句柄、没有 `async`：它交的是本轮**已经取好的值**
//! （事实切片 / 用户发言 / 读方主体 / 工具预算）。两个理由：
//!
//! 1. **事实是值**（DE-001 D-084）——同一轮内各读方看到的是同一刻的事实，不各开各的源；
//! 2. provider 因此是纯函数：可单测、不持状态、也不需要让 plugin-api 认识宿主的会话句柄
//!    （那会变成一条插件 → 宿主的反向依赖）。
//!
//! 代价写在这里：**provider 拿不到请求期才产生的东西**（例如本轮工具清单）。那类段
//! （委派者真源）因此还不是 provider——它依赖收集完成后的 `tools`，而收集是广播，
//! 遍历期取集合会漏掉尚未访问到的兄弟插件（见 `chat_loop::inputs` 的 ⑥′）。

use serde_json::Value;
use symbio_core::Event;

/// 已知的**段身份**（`meta.kind` 的取值）——宿主词汇，与 [`crate::behaviors`] 同层。
///
/// 段身份是 `&'static str` 而不是枚举：provider 来自任意插件，宿主不可能预先枚举完。
/// 这与 D-210 对行为名的裁决同形——**已知名字登记在这里，新名字由声明方自带**。
///
/// 与行为名不同的是：未知的段名**不会静默失效**。拼接方按视图次序取全部段，没有
/// 「按名字查一张表」的第二处拼写，所以拼错只影响它自己那一段的 `meta.kind` 标记。
pub mod slots {
    /// 长期记忆（背景事实，置顶）。
    pub const RECALL: &str = "recall_context";
    /// 任务调度候选集。
    pub const READYSET: &str = "readyset";
    /// 委派者真源（D-047）。它还不是 provider，理由见模块文档。
    pub const DELEGATE: &str = "delegate_context";

    /// 宿主**当前**认识的段身份。
    ///
    /// 只用于校验与判据读取面，不参与拼接（拼接按视图次序，不查表）。
    pub const KNOWN: &[&str] = &[RECALL, READYSET, DELEGATE];
}

/// 召回视图里的一行：过往会话的原文 + 它的认知标签。
///
/// 形状而不是 `RecallView`：那个类型住 `symbio-plugin-loop`（产出方与消费方都在那里），
/// plugin-api 不能也不该认识它。控制面把它**转成这一份**再交给 provider——转的是形状，
/// 判据（哪些条目可见、哪些技能被摘掉）已经在控制面做完。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecallLine<'a> {
    pub content: &'a str,
    pub tag: &'a str,
}

/// 本轮**已判过**的召回视图（见 [`RecallLine`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecallFacts<'a> {
    /// 召回是否降级（读侧预算耗尽等）——渲染方据此加一句说明。
    pub degraded: bool,
    pub lines: &'a [RecallLine<'a>],
}

/// 一次请求的**读侧值包**：provider 能看见的全部输入。
///
/// 全部是借用，构造一次、本轮各 provider 共用——与「事实是值」同一条口径。
#[derive(Debug, Clone, Copy)]
pub struct ContextRequest<'a> {
    /// 轮号。
    pub turn: u64,
    /// 本轮第几次请求（0 = 首幕）。分段判据若要看「这是不是第一次」，读它。
    pub request_round: u32,
    /// 读方主体：可见切片按它收放（不是写方——判的是 A、看的是 B 就没有隔离可言）。
    pub principal: &'a str,
    /// 本轮用户发言（语义维的输入；`None` = resume / 心跳这类没有新发言的请求）。
    pub query: Option<&'a str>,
    /// 本会话的事实切片（只读）。
    pub facts: &'a [Event],
    /// 已判过的召回视图（见 [`RecallFacts`]）。`None` = 本轮没召回。
    pub recall: Option<&'a RecallFacts<'a>>,
    /// 任务段的容量上限（控制面的配置投影，不是判据）。与调度无关的来源忽略它。
    pub task_cap: usize,
}

/// 一段要注入的文本。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextSection {
    /// 段身份（见 [`slots`]）。
    pub slot: &'static str,
    /// 正文。空串由拼接方过滤（判据只有一处：取值点）。
    pub text: String,
}

/// 能力对**自己的上下文贡献**的实现。
///
/// 一个 provider 实例 = 一个实现 + 一次 [`submit_context_provider!`](crate::submit_context_provider)；
/// 加一个实例只改
/// 能力包。返回空 = 本请求没有可注入的内容（**默认**：多数来源在多数轮次上是空的）。
pub trait ContextProvider: Sync {
    /// 装配顺序的依据：多个 provider 按 owner 字典序执行（DE-025 §3 第 4 条）。
    fn owner(&self) -> &'static str;

    /// 本请求要注入的段。可以给多段，也可以一段都不给。
    ///
    /// 只产文本，**不写事实**（写口唯一，I1）；也不参与判断（D-208）。
    fn provide(&self, _req: &ContextRequest<'_>) -> Vec<ContextSection> {
        Vec::new()
    }
}

/// 一条 provider 的链接期提交。
pub struct ContextProviderDecl {
    pub provider: &'static dyn ContextProvider,
}

inventory::collect!(ContextProviderDecl);

/// 提交一条上下文 provider。在插件 crate 的模块作用域调用一次即可。
///
/// ```ignore
/// symbio_plugin_api::submit_context_provider!(RecallProvider);
/// ```
#[macro_export]
macro_rules! submit_context_provider {
    ($p:expr) => {
        $crate::inventory::submit! {
            $crate::context::ContextProviderDecl { provider: &$p }
        }
    };
}

/// 已链接进来的全部 provider，**按 owner 字典序**（同 owner 保持提交序）。
///
/// 排序在读取侧做而不是装配期做一次：表是 `inventory` 的编译期集合，本仓不持有第二个
/// 可变注册表（与 `effects::handlers` 同一条纪律）。
pub fn providers() -> Vec<&'static dyn ContextProvider> {
    let mut all: Vec<&'static dyn ContextProvider> = inventory::iter::<ContextProviderDecl>()
        .map(|d| d.provider)
        .collect();
    all.sort_by_key(|p| p.owner());
    all
}

/// 收集一次请求的全部段：按 owner 序、按各 provider 的返回序。
///
/// 顺序确定（DE-025 §3 第 4 条）：段的位置就是模型读到的先后，而它只与 owner 字典序
/// 有关、与装配次序无关。
pub fn sections(req: &ContextRequest<'_>) -> Vec<ContextSection> {
    let mut out = Vec::new();
    for provider in providers() {
        out.extend(provider.provide(req));
    }
    out
}

/// 段的载荷形状：`meta.kind` 由 [`ContextSection::slot`] 单点给出。
///
/// 放在这里而不是让各拼接点自己 `json!`：拼写只有一处，且与 [`slots`] 同源。
pub fn section_meta(slot: &'static str) -> Value {
    serde_json::json!({ "kind": slot })
}

#[cfg(test)]
#[path = "context.test.rs"]
mod tests;
