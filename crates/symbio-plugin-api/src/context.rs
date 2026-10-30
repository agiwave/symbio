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
//! ## 两种节奏（D-220）
//!
//! 段的内容不都在发言落地时就齐：召回 / 调度只读开口前的读侧值包；委派者真源要的是
//! **工具清单收集成形之后**的产物（收集是广播，遍历期取集合会漏掉尚未访问到的
//! 兄弟插件）。因此端口分两种收集节奏：
//!
//! - [`Cadence::Utterance`]：一次发言收一次，provider 只读 [`ContextRequest`] 的常驻字段；
//! - [`Cadence::Request`]：每次模型请求、在工具收集之后，额外带 [`LiveInputs`]。
//!
//! 宿主按「发言级段在前、请求级段在后」拼一次请求前缀——两种段在这个数组里的先后
//! 是判据，不是实现细节。判断（例如 Q1「本轮该不该起 worker」）仍在控制面：
//! 请求级输入交的是**判完的结论**（[`DelegateVerdict`]），provider 只落笔。
//!
//! ## 预算与裁剪（D-243）
//!
//! 段不是白送的：每段占 token，段数由插件决定，宿主不能假设「拼上就一定放得下」。
//! provider 用 [`ContextSection::new`] 声明**优先级**，成本由宿主在收集后计量
//! （[`fit_sections`] 的注入闭包）——两格读数都挂在段上，裁剪只认这两格与收集次序，
//! 不按插件名 / 段名特判。放不下的段在这里被裁掉并计入 [`SectionFit`]，宿主据此出声
//! （裁了谁、超了多少）；「谁更重要」仍是声明方的判断，本模块不做第二遍。

use serde_json::Value;
use symbio_core::{CapabilityMeta, Event, WorkerProgress};

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
    /// 委派者真源（D-047）：请求级节奏——要的是收集完成后的工具清单（见 [`super::LiveInputs`]）。
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
    /// 来源事实的存储 `seq`——行的身份（摘行的读方按它点名，D-251：摘行按名字读折叠）。
    pub seq: u64,
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
/// 里面**没有轮号、没有配置结构体**（D-252）：要读开关就读 `facts` 里那条
/// `session.config` 快照，读不到就用出厂值。
#[derive(Debug, Clone, Copy)]
pub struct ContextRequest<'a> {
    /// 本轮第几次模型请求（0 = 第一次）。分段判据若要看「这是不是首幕」，读它。
    pub request_round: u32,
    /// 读方主体：可见切片按它收放（不是写方——判的是 A、看的是 B 就没有隔离可言）。
    pub principal: &'a str,
    /// 本轮用户发言（语义维的输入；`None` = resume / 心跳这类没有新发言的请求）。
    pub query: Option<&'a str>,
    /// 本会话的事实切片（只读）：开关与折叠的输入都在这里。
    pub facts: &'a [Event],
    /// 已判过的召回视图（见 [`RecallFacts`]）。`None` = 本轮没召回。
    pub recall: Option<&'a RecallFacts<'a>>,
    /// 请求级产物（只在 [`Cadence::Request`] 收集时存在；发言级收集为 `None`）。
    pub live: Option<&'a LiveInputs<'a>>,
}

/// 段的收集节奏：什么时候收集、能看见哪些请求期产物。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cadence {
    /// 一次发言收一次（首幕之前）：输入只有 [`ContextRequest`] 的常驻字段。
    Utterance,
    /// 每次模型请求、工具清单收集成形之后：额外带 [`LiveInputs`]。
    Request,
}

/// 请求级（[`Cadence::Request`]）才有的输入：收集完成后的本轮产物。
///
/// 这些值在发言级收集时还不存在——工具清单是收集的**产物**（遍历期取集合会漏掉尚未
/// 访问到的兄弟插件），worker 进展读自存储。宿主在收集完成后一次取齐、按值递入。
#[derive(Debug, Clone, Copy)]
pub struct LiveInputs<'a> {
    /// 本轮最终发给模型的工具清单（收集完成后的快照，与请求体是同一份）。
    pub tools: &'a [CapabilityMeta],
    /// 委派判定（Q1）的结论——控制面判完，provider 只落笔。
    pub delegate: DelegateVerdict<'a>,
    /// 各 worker 的进展（控制面从存储侧取好的值）。
    pub workers: &'a [WorkerProgress],
}

/// 委派判定（Q1）的结论。
///
/// 判据（前缀 / 关键词 / 阈值）与配置仍在控制面：provider 拿不到配置面，也不该
/// 重判（D-208）。三态必须分开——「判了要动」「判了不动」都要落事实（可审计），
/// 而「压根没判」（无用户新发言）一个字节都不该写。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DelegateVerdict<'a> {
    /// 本次请求没有用户新发言（resume / 心跳），判定无从谈起。
    NotJudged,
    /// 已判：应启动 worker，带可审计的理由码。
    Start { reason: &'a str },
    /// 已判：未命中任何委派判据。
    Idle,
}

/// 一段要注入的文本，带裁剪要用的两格读数（D-243）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextSection {
    /// 段身份（见 [`slots`]）。
    pub slot: &'static str,
    /// 正文。空串由拼接方过滤（判据只有一处：取值点）。
    pub text: String,
    /// 正文的 token 成本。**唯一写点是宿主**：provider 经 [`ContextSection::new`]
    /// 构造时填 0（= 未计量），[`fit_sections`] 用宿主注入的计量回填——成本必须与
    /// 预算同一把尺，让 provider 各写一份估算就成了第二套计量。
    pub cost: usize,
    /// 裁剪优先级：数大者先留。
    ///
    /// 与收集次序判的不是一件事：次序是**模型读到的先后**（由 owner 字典序决定，
    /// DE-025 §3 第 4 条），本值只在放不下时决定**先丢谁**。
    pub prio: u8,
}

impl ContextSection {
    /// provider 侧的构造口：成本留 0，等宿主计量。
    pub fn new(slot: &'static str, text: String, prio: u8) -> Self {
        Self {
            slot,
            text,
            cost: 0,
            prio,
        }
    }
}

/// 段裁剪的读数（D-243）：预算多少、要了多少、留下了多少、裁掉了谁。
///
/// 报告是**读数**不是判据：裁剪规则只认 [`ContextSection::prio`] 与收集次序，
/// 不按插件名 / 段名特判——否则「放不下时谁先走」就成了按名字硬编码的规则。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SectionFit {
    /// 本次可用的 token 预算。
    pub budget: usize,
    /// 全部段成本之和（裁剪前）。
    pub demanded: usize,
    /// 留下段的成本之和。
    pub kept_cost: usize,
    /// 被裁掉的段：`(段身份, 成本)`，按收集次序。
    pub dropped: Vec<(&'static str, usize)>,
}

impl SectionFit {
    /// 是否发生了裁剪。
    pub fn trimmed(&self) -> bool {
        !self.dropped.is_empty()
    }

    /// 超界量：需求超出预算多少 token（未超界为 0）。
    pub fn over(&self) -> usize {
        self.demanded.saturating_sub(self.budget)
    }
}

/// 按预算确定性裁剪（D-243）：`(prio 降序, 收集下标升序)` 逐段贪心。
///
/// 放不下的段**跳过而不是停**：更高优先级的段已经先试过，剩下的余量让更小的段
/// 继续试，能留一段是一段。输出保持**收集次序**——段的先后是模型读到的次序，
/// 与裁决次序无关。
///
/// `cost_of` 是宿主注入的计量（token）；[`ContextSection::cost`] 在这里被回填。
/// 计量器由调用方给而不是本模块自带：插件面不认识宿主的 tokenizer，而成本必须
/// 与预算同一把尺（见字段文档）。
pub fn fit_sections<F>(
    sections: Vec<ContextSection>,
    budget: usize,
    cost_of: F,
) -> (Vec<ContextSection>, SectionFit)
where
    F: Fn(&str) -> usize,
{
    let mut sections = sections;
    for s in &mut sections {
        s.cost = cost_of(&s.text);
    }

    let demanded = sections.iter().map(|s| s.cost).sum();
    let mut order: Vec<usize> = (0..sections.len()).collect();
    order.sort_by(|&a, &b| sections[b].prio.cmp(&sections[a].prio).then(a.cmp(&b)));

    let mut keep = vec![false; sections.len()];
    let mut kept_cost = 0usize;
    for i in order {
        if kept_cost + sections[i].cost <= budget {
            kept_cost += sections[i].cost;
            keep[i] = true;
        }
    }

    let dropped = sections
        .iter()
        .zip(&keep)
        .filter(|(_, k)| !**k)
        .map(|(s, _)| (s.slot, s.cost))
        .collect();
    let fit = SectionFit {
        budget,
        demanded,
        kept_cost,
        dropped,
    };
    let kept = sections
        .into_iter()
        .zip(keep)
        .filter_map(|(s, k)| k.then_some(s))
        .collect();
    (kept, fit)
}

/// 能力对**自己的上下文贡献**的实现。
///
/// 一个 provider 实例 = 一个实现 + 一次 [`submit_context_provider!`](crate::submit_context_provider)；
/// 加一个实例只改
/// 能力包。返回空 = 本请求没有可注入的内容（**默认**：多数来源在多数轮次上是空的）。
pub trait ContextProvider: Sync {
    /// 装配顺序的依据：多个 provider 按 owner 字典序执行（DE-025 §3 第 4 条）。
    fn owner(&self) -> &'static str;

    /// 收集节奏（默认发言级）。需要请求期产物的 provider 声明 [`Cadence::Request`]，
    /// 并从 `req.live` 取用——发言级收集时它是 `None`。
    fn cadence(&self) -> Cadence {
        Cadence::Utterance
    }

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
/// 有关、与装配次序无关。`cadence` 只决定**问哪些 provider**——两种节奏的段由宿主
/// 按调用次序拼成一个数组（发言级段在前、请求级段在后）。
pub fn sections(req: &ContextRequest<'_>, cadence: Cadence) -> Vec<ContextSection> {
    let mut out = Vec::new();
    for provider in providers() {
        if provider.cadence() != cadence {
            continue;
        }
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
