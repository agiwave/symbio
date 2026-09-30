//! `governance` —— 权限与可见性（v2 ⑥，[plan/01 §7](../../../../docs/plan/01-核心架构.md)）。
//!
//! ## 为什么第零天就位
//!
//! 权限是**静默失效**的——违反时没有测试会失败（J3）。事后补，代价是重写所有
//! 历史事件（[plan/01 §7](../../../../docs/plan/01-核心架构.md)）。所以 S3 一阶
//! 只做两件事：授权矩阵（第 7 步）与默认隔离（第 8 步），其余一切等它。
//!
//! ## 授权必须成对（01 §7 的核心设计）
//!
//! | 侧 | 函数 | 缺失后果 |
//! |---|---|---|
//! | 写侧 | [`PermissionMatrix::can_write`] | 只有读侧 = 能给自己授权看一切 |
//! | 读侧 | [`PermissionMatrix::can_see`] | 只有写侧 = 看得到一切 |
//!
//! 因此矩阵**构造时**就拒绝不成对的主体策略：只有写侧 → 拒绝（出口判据，
//! [plan/04 §3](../../../../docs/plan/04-工程落地.md) 第 7 步）；查询对未知主体
//! 一律 fail-closed——「没说允许」就是拒绝，不是放行。
//!
//! ## 与 `capability` 域的同名辨析
//!
//! 本域的 [`Capability`]（权限矩阵的键，7 个封顶）与 `capability` 域的
//! `Capability`（LLM 可见工具描述符）是**两个概念**：前者回答「有权做什么」，
//! 后者回答「暴露什么工具」。计划文本同名（01 §7），代码里以**路径限定**消歧：
//! 本枚举只经 `governance::Capability` 引用，**不进根平铺**——两个 `Capability`
//! 同时出现在调用点，正是命名纪律要阻止的错位。
//!
//! ## 依赖纪律（[plan/05 §3.1](../../../../docs/plan/05-模块架构.md) ⑥ 行）
//!
//! 只依赖类型定义 + ④ store 的**读面**（事件切片），只读写事件——不持有
//! ①②③⑤ 的任何句柄。

/// 权限能力（[plan/01 §7](../../../../docs/plan/01-核心架构.md)：**7 个封顶**）。
///
/// 按「权威动作的种类」划分，不按「有多少个智能体」划分——新主体不新增能力，
/// 新动作种类才走 ADR（03 §3）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capability {
    /// 判决意图（快速档分类）。
    JudgeIntent,
    /// 首响（一个 turn 的第一句话）。
    ReplyFirst,
    /// 追加发言（对既有对话通道的后续写入）。
    ReplyAppend,
    /// 定义工作（任务分解、派活）。
    DefineWork,
    /// 产生产物。
    ProduceArtifact,
    /// 断言验证（验证者 ≠ 产出者，由 grants 表保证）。
    AssertVerification,
    /// 指派工作。
    AssignWork,
}

/// 可见域（[plan/01 §8](../../../../docs/plan/01-核心架构.md) 参数表：3 个取值）。
///
/// **C10**（[plan/04 §4](../../../../docs/plan/04-工程落地.md)）：默认
/// `thread_private`——默认隔离是**缺省值**，不是需要每个主体记得配置的东西。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum VisScope {
    /// 仅本线程可见（默认）。
    #[default]
    ThreadPrivate,
    /// 对矩阵内主体可见。
    Shared,
    /// 完全公开。
    Public,
}

/// 一个主体的完整策略：写侧（能力集合）+ 读侧（可见域）——**缺一不可**。
///
/// `vis_scope` 用 `Option` 表达「读侧未配置」：`None` 在构造矩阵时即被拒绝
/// （成对性），不会被静默当作某个默认值——「没说允许」必须显式说，默认值
/// （C10 `thread_private`）只作用于**内容**的缺省可见域，不替主体补授权。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrincipalPolicy {
    /// 身份（数据，任意字符串）。
    pub principal: String,
    /// 写侧：持有的能力。
    pub grants: Vec<Capability>,
    /// 读侧：可见域。`None` = 读侧未配置 ⇒ [`PermissionMatrix::new`] 拒绝。
    pub vis_scope: Option<VisScope>,
}

impl PrincipalPolicy {
    /// 读写成对的便捷构造。
    pub fn paired(
        principal: impl Into<String>,
        grants: Vec<Capability>,
        vis_scope: VisScope,
    ) -> Self {
        PrincipalPolicy {
            principal: principal.into(),
            grants,
            vis_scope: Some(vis_scope),
        }
    }
}

/// 成对性违规：策略只有一侧。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairingViolation {
    /// 只有写侧（有 grants，读侧未配置）——拒绝。
    WriteOnly { principal: String },
    /// 只有读侧（无 grants）——拒绝。
    ReadOnly { principal: String },
}

/// 授权矩阵（S3 第 7 步产物）。
///
/// 构造即校验成对性；查询 **fail-closed**：未知主体、未持有的能力、越界的
/// 可见域，一律拒绝。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionMatrix {
    policies: Vec<PrincipalPolicy>,
}

impl PermissionMatrix {
    /// 由主体策略集构造。**任何一侧缺失即整体拒绝**（出口判据「只有写侧 → 拒绝」，
    /// [plan/04 §3](../../../../docs/plan/04-工程落地.md) 第 7 步）。
    pub fn new(policies: Vec<PrincipalPolicy>) -> Result<Self, PairingViolation> {
        for p in &policies {
            if p.grants.is_empty() {
                return Err(PairingViolation::ReadOnly {
                    principal: p.principal.clone(),
                });
            }
            if p.vis_scope.is_none() {
                return Err(PairingViolation::WriteOnly {
                    principal: p.principal.clone(),
                });
            }
        }
        Ok(PermissionMatrix { policies })
    }

    /// 写侧：主体持有的全部能力（空切片 = 未知主体 / 只读被拒后不存在）。
    pub fn grants_of(&self, principal: &str) -> &[Capability] {
        self.policies
            .iter()
            .find(|p| p.principal == principal)
            .map(|p| p.grants.as_slice())
            .unwrap_or(&[])
    }

    /// 写侧判定：主体是否可以权威地做 `cap`。**fail-closed**。
    pub fn can_write(&self, principal: &str, cap: Capability) -> bool {
        self.grants_of(principal).contains(&cap)
    }

    /// 读侧：主体的可见域（未知主体 ⇒ `None` ⇒ [`Self::can_see`] 必拒）。
    pub fn sees_of(&self, principal: &str) -> Option<VisScope> {
        self.policies
            .iter()
            .find(|p| p.principal == principal)
            .and_then(|p| p.vis_scope)
    }

    /// 读侧判定：`viewer` 能否看到 `owner` 的、可见域为 `scope` 的内容。
    ///
    /// - `thread_private`：仅 owner 本人（**默认隔离**，C10——越界读取 0 的构造侧）；
    /// - `shared`：矩阵内任何主体；
    /// - `public`：任何人。
    pub fn can_see(&self, viewer: &str, owner: &str, scope: VisScope) -> bool {
        match scope {
            VisScope::ThreadPrivate => viewer == owner,
            VisScope::Shared => self.sees_of(viewer).is_some(),
            VisScope::Public => true,
        }
    }
}

#[cfg(test)]
#[path = "mod.test.rs"]
mod tests;
