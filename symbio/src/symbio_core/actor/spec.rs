//! [`ActorSpec`] —— 一行主体规格，及其两个枚举字段。
//!
//! 字段取值**全部反推自现行 `chat_loop`**（见模块文档的表）。这里不引入任何
//! 现行代码里没有的概念：`principal` 就是会话 id 字符串（**不新建 `Principal`
//! 结构**，留给 S08 多主体时再收）、`budget_ms` 沿用 `0 = 不限` 的现行语义。

use serde::{Deserialize, Serialize};

/// Actor 的**行为模式** —— 它"以什么方式"工作。
///
/// 四个取值直接对应本方案要登记的四个执行者（`docs/plan/06` §2.3 的表）：
/// 现行 `chat_loop` 是 [`Reasoner`](Self::Reasoner)，检索者/判定者/验证者由
/// 后续增量登记。**枚举比字符串好**：写错一个模式名是编译错误，不是静默不生效。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorPattern {
    /// 推理者：现行 `chat_loop` 的形态（多轮 LLM + 工具循环）
    Reasoner,
    /// 翻译者：把查询翻译成检索/召回动作（S06 检索者的形态）
    Translator,
    /// 判定者：短预算、只做裁决（S07 抢占判定的形态）
    Decider,
    /// 验证者：与产出者不同主体，做独立核验（S03/S08 不自验的形态）
    Verifier,
}

impl ActorPattern {
    /// 线上形式（`serde` 用的同一份字符串）。
    ///
    /// 与 `#[serde(rename_all = "snake_case")]` 必须一致——单元测试
    /// `pattern_wire_matches_serde` 把两者钉在一起，改一处忘另一处会失败。
    pub const fn wire(self) -> &'static str {
        match self {
            ActorPattern::Reasoner => "reasoner",
            ActorPattern::Translator => "translator",
            ActorPattern::Decider => "decider",
            ActorPattern::Verifier => "verifier",
        }
    }

    /// 全部取值（审计者 / 测试遍历用；顺序稳定）。
    pub const ALL: [ActorPattern; 4] = [
        ActorPattern::Reasoner,
        ActorPattern::Translator,
        ActorPattern::Decider,
        ActorPattern::Verifier,
    ];
}

/// Actor 的**作用域** —— 它属于哪一层装配。
///
/// 与 [`ASSEMBLY_SUB_AGENT_PLUGINS`](crate::symbio_core::assembly::ASSEMBLY_SUB_AGENT_PLUGINS)
/// 的分形层级同构：root 会话是 [`Root`](Self::Root)，子智能体树是
/// [`SubAgent`](Self::SubAgent)。
///
/// 这是新断言 **A5** 的落点：登记行声明的 `scope` 必须与登记者所在装配层一致——
/// 越层的行**登记不进去**（登记入口是
/// [`ActorSource::register_all`](crate::symbio_core::ActorSource::register_all)，
/// 层由 [`layer`](crate::symbio_core::ActorSource::layer) 给出）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "agent_id")]
pub enum ActorScope {
    /// 系统树 / root 会话
    Root,
    /// 子智能体树（携带 agent_id，即该层目录名）
    SubAgent(String),
}

impl ActorScope {
    /// 线上形式：`"root"` 或 `"sub_agent:<id>"`（单字符串，便于 `actor/list` 平铺展示）。
    ///
    /// 与 `serde` 的 tag/content 形态不同——那是**结构化**形式（给程序读），
    /// 这是**扁平**形式（给人看、给 list 路由返回）。两者都由单元测试钉住。
    pub fn wire(&self) -> String {
        match self {
            ActorScope::Root => "root".to_string(),
            ActorScope::SubAgent(id) => format!("sub_agent:{id}"),
        }
    }

    /// 是否 root 层。
    pub fn is_root(&self) -> bool {
        matches!(self, ActorScope::Root)
    }
}

/// 一行主体规格 —— **执行一个 Actor 需要的全部参数**。
///
/// 字段 `pub`：它是**只读描述**（同 [`Fact`](crate::symbio_core::Fact) 的处理），构造经
/// [`ActorSpec::new`]。没有 setter——"改一行 Actor" = 重新登记一行，不是就地改。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActorSpec {
    /// 主体标识。root 会话即 `session_id`。
    ///
    /// **登记期的占位符**：`session` 插件登记的 reasoner 行写 `principal = ""`，
    /// 因为登记发生在**任何会话存在之前**；运行期由
    /// [`ActorSpec::resolve`] 以真实 `session_id` 覆盖。
    pub principal: String,
    /// 行为模式（见 [`ActorPattern`]）。
    pub pattern: ActorPattern,
    /// 墙钟预算（毫秒）。`0` = **不限**（与现行 `max_tool_rounds = 0 = 无限轮次`
    /// 同构，不是魔法数）。
    pub budget_ms: u64,
    /// 作用域（见 [`ActorScope`]）。
    pub scope: ActorScope,
    /// 唯一名称（如 `session.reasoner`）。表中按它寻址，**不参与**主体语义——
    /// 它是"这一行的标识"，不是"这个 Actor 的身份"（身份是 `principal`）。
    pub name: String,
}

impl ActorSpec {
    /// 构造一行。**唯一入口**——`name` 与其余四字段一起给定，不存在"半行"。
    pub fn new(
        name: impl Into<String>,
        principal: impl Into<String>,
        pattern: ActorPattern,
        budget_ms: u64,
        scope: ActorScope,
    ) -> Self {
        Self {
            name: name.into(),
            principal: principal.into(),
            pattern,
            budget_ms,
            scope,
        }
    }

    /// **内置默认行** —— 表为空时执行器用的那一行，取值**就是现行 `chat_loop`**。
    ///
    /// 这是 J2 的机制基础：没有 [`ActorSource`](super::ActorSource) 登记任何行时，
    /// 执行器拿到的仍然是这一行，会话行为与改动前**逐字节相同**。
    /// 于是"表"是**扩展点**，不是**运行时必需品**。
    pub fn builtin(name: impl Into<String>) -> Self {
        Self::new(name, "", ActorPattern::Reasoner, 0, ActorScope::Root)
    }

    /// 运行期解析：用真实 `principal` 覆盖登记期的占位符，得到本轮的生效行。
    ///
    /// 为什么不在登记期填 `principal`：登记发生在装配期（任何会话之前），
    /// 而 `principal` 只有在**开跑一轮**时才知道。占位符 + 覆盖是这两者之间
    /// 唯一诚实的表达——而不是在登记期写个假的 id 再假装它是对的。
    pub fn resolve(&self, principal: impl Into<String>) -> Self {
        Self {
            principal: principal.into(),
            ..self.clone()
        }
    }

    /// 预算是否"不限"。
    pub fn is_unbudgeted(&self) -> bool {
        self.budget_ms == 0
    }

    /// 线上形式（`actor/list` 路由返回的形状）。字段名与结构体一致，便于核对。
    pub fn to_wire(&self) -> serde_json::Value {
        serde_json::json!({
            "name": self.name,
            "principal": self.principal,
            "pattern": self.pattern.wire(),
            "budget_ms": self.budget_ms,
            "scope": self.scope.wire(),
        })
    }
}
