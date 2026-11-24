//! 沙箱机制：一个**横切的通用能力句柄**，与 hook 同形（D-531）。
//!
//! ## 本模块只定义「形状」，不含任何策略
//!
//! 沙箱有两个正交维度（讨论稿 §1.5）：执行隔离（转发式拦截）与能力约束
//! （查询式）。两者的核心机制不同——这正是本模块要**屏蔽**的差异：
//! 架构看到的是同一个句柄 [`SandboxFace`]，至于它背后是把载荷转发进虚拟环境、
//! 还是按用户配的黑白名单匹配事实，全是沙箱插件的内部实现。
//!
//! 因此本模块遵守三条硬约束（与全仓插件化原则一致）：
//!
//! 1. **不认识工具**：没有工具名清单、没有操作 op 常量、没有「该不该拦某工具」的分支；
//! 2. **不持有策略**：黑白名单、风险阈值、虚拟环境配置都住在沙箱插件里，是用户数据；
//! 3. **不暴露配置**：其它插件只拿到 [`SandboxFace`] 句柄，看不到任何沙箱内部配置项。
//!
//! ## 两个方法对应两个维度
//!
//! | 方法 | 维度 | 机制 | 返回 |
//! |---|---|---|---|
//! | [`SandboxFace::intercept`] | 执行隔离（where） | 转发式拦截，载荷不透明透传 | `None` 放行 / `Some(载荷)` 拦下 |
//! | [`SandboxFace::permits`] | 能力约束（what） | 查询式，工具提交意图事实 | `bool` 许可 / 不许可 |
//! | [`SandboxFace::approval_params`] | 会话承接（裁决 B） | 只读查询，装配期取一次 | `(模式, 阈值)` 通用字符串 |
//!
//! `ask`（询问用户）**不在工具侧实现**：工具查询只拿 bool；需要用户确认时，
//! 由沙箱插件在拦截点统一产出审批载荷。这是用户裁决的精确化（讨论稿 §8.1）。
//!
//! ## ctx 键为什么是共享槽
//!
//! 与 [`crate::agent::AGENT_PERMS`] 同形：容器对每个参与 traverse 的插件各 fork
//! 一份 ctx，发布方（sandbox 插件）和消费方（tool / 各工具）在收集期彼此不可见，
//! 因此用 `Arc<OnceLock<..>>` 做一次写入的共享槽：
//! 收集方在 traverse 前 [`sandbox_init_slot`]、发布方在 traverse 里 [`sandbox_publish`]、
//! 收集结束后由会话把同一个 Arc 挂回轮 ctx，执行期 [`sandbox_of`] 即可读到。
//!
//! **缺席语义**：没有槽 / 槽是空的 = 本会话没有沙箱 ⇒ 全放行（消费方自行降级，
//! 并保留既有的「无默认沙箱、批准不等于隔离」诚实提示）。

use std::sync::{Arc, OnceLock};

use serde_json::Value;
use symbio_kernel::keys::SymbioKey;
use symbio_kernel::plugin::{PluginInvokeRequest, PluginInvokeRequestExt};

/// 一次**意图事实**：工具解析自己的参数后提交给沙箱的语义载荷（讨论稿 §1.6）。
///
/// `kind` 是开放字符串——工具自报新种类无需改本模块或沙箱，沙箱按 kind 配规则；
/// `value` 是工具**归一化后**的值（如命令已经拆好子命令、路径已经归一化），
/// 沙箱只做匹配，不解析任何工具语法。
///
/// 这与被否定的 `CapabilityMeta.operation` 本质不同：operation 声明的是**工具身份**
/// （让沙箱反过来认识工具），这里声明的是**数据形状**（工具把自己的语义喂给沙箱）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxFact {
    /// 事实种类（开放词表，如 `shell_command` / `vdfs_path` / `url`）。
    pub kind: String,
    /// 归一化后的语义值（工具算好，沙箱不解析）。
    pub value: String,
}

impl SandboxFact {
    /// 造一条事实。开放字符串直接收，不做任何词表校验——词表属于沙箱插件的策略。
    pub fn new(kind: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            value: value.into(),
        }
    }
}

/// 一次调用在同一档案快照内得到的完整裁决（D-533）。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SandboxDecision {
    pub permitted: bool,
    /// 规则给出的最低风险；消费方不得用它调低工具自身风险。
    pub risk: Option<String>,
    pub intercept: Option<Value>,
    pub approval_mode: Option<String>,
    pub risk_threshold: Option<String>,
    pub profile_version: u64,
    pub reason: Option<String>,
}

impl SandboxDecision {
    pub fn denied(reason: impl Into<String>) -> Self {
        Self {
            permitted: false,
            reason: Some(reason.into()),
            ..Self::default()
        }
    }
}

/// 沙箱向其它插件提供的**通用能力句柄**（与 hook 同形，机制侧不认业务）。
///
/// 实现方是沙箱插件；消费方是会话/编排层（拦截）与工具执行体（查询）。
/// 消费方只依赖这个 trait，看不到任何沙箱实现、配置项或工具清单。
pub trait SandboxFace: Send + Sync {
    /// 工具查询：提交本次调用的意图事实，问「这一笔许可吗」。
    ///
    /// **只返回 bool**：`ask` 由沙箱在拦截点统一产出，工具不实现询问逻辑。
    /// 无沙箱句柄时消费方全放行——缺席语义由消费方保证（见模块首注释）。
    fn permits(&self, facts: &[SandboxFact]) -> bool;

    /// 规则风险档查询：`permits` 放行后，若这批事实命中了沙箱配置的某条**允许规则**
    /// 且该规则声明了风险档，返回通用档位字符串 `low` / `medium` / `high`。
    ///
    /// 这与 [`Self::permits`] 的分工是「许可」与「几档风险」：`permits` 仍只返回 bool，
    /// 本方法只是可选细化——`None` = 沙箱不指定，工具继续用自己的风险判定。
    /// 返回的是**通用字符串**而非沙箱配置结构，调用方看不到任何黑白名单 / 匹配模式。
    ///
    /// 不是 `ask`：询问用户仍统一在沙箱拦截点产出，这里只回答风险档。
    fn risk_of(&self, _facts: &[SandboxFact]) -> Option<String> {
        None
    }

    /// 拦截点：会话/编排层在工具执行前调用。
    ///
    /// `tool_name` / `args` 是**不透明透传**——本 trait 与调用方都不认识工具名，
    /// 也不解释载荷；沙箱插件内部决定是转发进虚拟环境、还是产出审批/拒绝载荷。
    /// `None` = 放行；`Some(载荷)` = 拦下并把载荷交回调用方。
    fn intercept(&self, tool_name: &str, args: &Value) -> Option<Value>;

    /// 本句柄绑定的沙箱档案的**审批参数**（`审批模式`, `风险阈值`），供装配期写进
    /// 通用 `MODE` / `RISK_LEVEL` ctx 键（裁决 B：会话只选 sandbox_id，语义由沙箱承接）。
    ///
    /// 返回的是引擎**算出的通用值**（两个普通字符串），不是沙箱配置结构——调用方
    /// 看不到白名单、工具清单或任何内部开关。`None` = 该档案没配这一项，装配侧
    /// 回落默认。缺席语义：没有句柄 = 装配侧保留 metadata 旧值或默认（全放行降级）。
    ///
    /// 这是一个**只读查询**：`ask`/审批载荷不在工具侧实现，工具查询只拿 bool，
    /// 询问统一由沙箱在拦截点产出（与 [`Self::permits`] 同一裁决）。
    fn approval_params(&self) -> (Option<String>, Option<String>) {
        (None, None)
    }

    /// 一次读取完成本次调用的全部裁决；执行链应优先消费该方法，避免跨快照组合。
    fn decide(&self, facts: &[SandboxFact], tool_name: &str, args: &Value) -> SandboxDecision {
        let intercept = self.intercept(tool_name, args);
        let permitted = self.permits(facts) && intercept.is_none();
        let (approval_mode, risk_threshold) = self.approval_params();
        SandboxDecision {
            permitted,
            risk: self.risk_of(facts),
            intercept,
            approval_mode,
            risk_threshold,
            reason: (!permitted).then(|| "sandbox denied the call".to_string()),
            ..SandboxDecision::default()
        }
    }
}

/// 沙箱句柄的 ctx Key —— **键与值同住**（D-095）。
///
/// 值类型 `Arc<OnceLock<Arc<dyn SandboxFace>>>` 是本域类型，故键住本模块。
/// `WIRE = false`：trait object 无进程外形态，只在进程内传递。
pub struct SandboxKey;
impl SymbioKey for SandboxKey {
    type Value = Arc<OnceLock<Arc<dyn SandboxFace>>>;
    const WIRE: bool = false;
    fn name(&self) -> &'static str {
        "sandbox"
    }
    fn parse(&self, _s: &str) -> Option<Self::Value> {
        None
    }
    fn format(&self, _v: &Self::Value) -> String {
        self.name().to_string()
    }
}
/// 沙箱句柄键的唯一实例。
pub const SANDBOX: SandboxKey = SandboxKey;

/// 收集方在 traverse 前建槽（幂等：已有槽就复用，与 `AGENT_PERMS` 同形）。
///
/// 发布方（沙箱插件）也自备一遍：两个入口共用这一幂等建槽，与能力错误桶同一形态。
pub fn sandbox_init_slot(ctx: &Arc<dyn PluginInvokeRequest>) {
    if ctx.get(SANDBOX).is_none() {
        ctx.set(SANDBOX, Arc::new(OnceLock::new()));
    }
}

/// 发布方（沙箱插件）在能力收集期把自己的实现写进共享槽。
///
/// `OnceLock::set` 只成功一次：第二个发布方会被拒绝（返回 `false`），
/// 不静默顶掉已发布的句柄——与端口判重「同名冲突即不可执行」同一纪律。
pub fn sandbox_publish(ctx: &Arc<dyn PluginInvokeRequest>, face: Arc<dyn SandboxFace>) -> bool {
    sandbox_init_slot(ctx);
    ctx.get(SANDBOX)
        .and_then(|slot| slot.set(face).ok())
        .is_some()
}

/// 消费方读沙箱句柄。**没有槽 / 槽是空的 = 本会话没有沙箱** ⇒ 全放行降级。
pub fn sandbox_of(ctx: &dyn PluginInvokeRequest) -> Option<Arc<dyn SandboxFace>> {
    ctx.get(SANDBOX).and_then(|slot| slot.get().cloned())
}

#[cfg(test)]
mod tests;
