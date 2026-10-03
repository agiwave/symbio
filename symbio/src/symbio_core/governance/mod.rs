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
//! ## 生产接线（[04 §3.1 批⑥](../../../../docs/plan/04-工程落地.md)）
//!
//! 矩阵**构造**在宿主装配面 `crate::authz`（本机主体清单），**判定**在
//! `plugins/session` 两处：
//!
//! | 侧 | 调用点 | 判什么 |
//! |---|---|---|
//! | 写侧 | `v2_bridge` 收束入格前 | `can_reply`（首条 → `ReplyFirst`，后续 → `ReplyAppend`） |
//! | 读侧 | `stats` 按读方声明判（属主 = `SESSION_OWNER`，部署事实） | `can_see`（`thread_private` 缺省，C10）⇒ 非属主读数为空 |
//!
//! 为什么表住宿主而不是 core 或插件：core 拥有**机制**（闭集与判定），不知道
//! 「谁」；插件互不可见（E-009），任一插件持有它都会逼别的插件跨插件引用。
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

impl Capability {
    /// 能力名（授权表的书写形态）→ 枚举。**认不出的名 = `None`**，由
    /// [`PermissionMatrix::from_names`] 拒绝构造——不把拼写错误静默读成
    /// 「没有这个能力」，那等于把授权表的笔误变成一条悄悄生效的拒绝。
    fn from_name(name: &str) -> Option<Capability> {
        Some(match name {
            "judge.intent" => Capability::JudgeIntent,
            "reply.first" => Capability::ReplyFirst,
            "reply.append" => Capability::ReplyAppend,
            "define.work" => Capability::DefineWork,
            "produce.artifact" => Capability::ProduceArtifact,
            "assert.verification" => Capability::AssertVerification,
            "assign.work" => Capability::AssignWork,
            _ => return None,
        })
    }
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
    /// 同一主体同时持有 `DefineWork` 与 `AssertVerification`（C15：不自验）——拒绝。
    /// 验证者 ≠ 产出者不是纪律是构造期约束：双持的主体给自己的产物盖章，
    /// 质量闸门等于没有（S7 第 17 步，[roadmap/S03 §5](../../../../docs/plan/roadmap/S03-多步任务与返工.md)）。
    SelfVerifier { principal: String },
}

/// 授权表构造失败：认不出的能力名 / 不成对的策略——**拒绝构造，不是警告**。
///
/// 两个变体都不该在运行期出现（表是宿主装配期的常量），但它们必须**报出来**：
/// 静默降级会让「授权表写错了」变成「某次请求被拒了」，两者要修的地方完全不同。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyError {
    /// 授权表写了认不出的能力名（拼写，或新增能力没登记进 [`Capability`] 闭集）。
    UnknownCapability { principal: String, name: String },
    /// 成对性 / C15 / S9 违规（见 [`PairingViolation`]）。
    Pairing(PairingViolation),
}

impl std::fmt::Display for PolicyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PolicyError::UnknownCapability { principal, name } => {
                write!(
                    f,
                    "主体 `{principal}` 的能力名 `{name}` 认不出来（7 项闭集）"
                )
            }
            PolicyError::Pairing(v) => match v {
                PairingViolation::WriteOnly { principal } => {
                    write!(f, "主体 `{principal}` 只配了写侧（缺 `vis_scope`）")
                }
                PairingViolation::ReadOnly { principal } => {
                    write!(f, "主体 `{principal}` 只配了读侧（缺 grants）")
                }
                PairingViolation::SelfVerifier { principal } => {
                    write!(f, "主体 `{principal}` 双持产出与验证（C15 不自验）")
                }
            },
        }
    }
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
            // C15（不自验）：DefineWork + AssertVerification 不得同持（S7 第 17 步）；
            // 高风险组合（S9，[roadmap/S09 §5](../../../../docs/plan/roadmap/S09-外部执行与熔断.md)
            // 验收 3）：外部执行（ProduceArtifact）+ AssertVerification 同样不得同持——
            // 给自己的外部动作盖章 = 质量闸门等于没有。
            let verifies = p.grants.contains(&Capability::AssertVerification);
            let self_verifies = verifies
                && (p.grants.contains(&Capability::DefineWork)
                    || p.grants.contains(&Capability::ProduceArtifact));
            if self_verifies {
                return Err(PairingViolation::SelfVerifier {
                    principal: p.principal.clone(),
                });
            }
        }
        Ok(PermissionMatrix { policies })
    }

    /// 由**能力名**表构造（宿主装配面用：插件与宿主不持有 7 项能力枚举，
    /// 两个同名 `Capability` 的命名纪律见 [`Capability`] 与根出口注释）。
    ///
    /// 每行 = `(主体, 能力名切片, 可见域)`。闭集校验仍在这里做：认不出的名字
    /// **拒绝整体构造**（[`PolicyError::UnknownCapability`]），不降级为
    /// 「这行没这个能力」——那会把授权表的笔误变成一条悄悄生效的拒绝。
    pub fn from_names(rows: &[(&str, &[&str], VisScope)]) -> Result<Self, PolicyError> {
        let mut policies = Vec::with_capacity(rows.len());
        for (principal, names, vis_scope) in rows {
            let mut grants = Vec::with_capacity(names.len());
            for &name in *names {
                match Capability::from_name(name) {
                    Some(cap) => grants.push(cap),
                    None => {
                        return Err(PolicyError::UnknownCapability {
                            principal: (*principal).to_string(),
                            name: name.to_string(),
                        })
                    }
                }
            }
            policies.push(PrincipalPolicy::paired(
                (*principal).to_string(),
                grants,
                *vis_scope,
            ));
        }
        Self::new(policies).map_err(PolicyError::Pairing)
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

    /// 写侧判定（收束闸）：主体能否**收束一轮发言**——`v2_bridge` 入格前的判据。
    ///
    /// `first` = 该轮尚无收束格：首条 → [`Capability::ReplyFirst`]，后续（重试 /
    /// 追加）→ [`Capability::ReplyAppend`]。「轮次位置 → 能力」的映射只在这一处
    /// 定义，调用点不复述（口径只活在 core）。未持有 ⇒ 拒绝入格（fail-closed）。
    pub fn can_reply(&self, principal: &str, first: bool) -> bool {
        let cap = if first {
            Capability::ReplyFirst
        } else {
            Capability::ReplyAppend
        };
        self.can_write(principal, cap)
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
