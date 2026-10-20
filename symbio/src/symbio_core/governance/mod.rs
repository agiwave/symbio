//! `governance` —— 权限与可见性（v2 ⑥，[plan/01 §7](../../../../docs/004-plan/001-核心架构.md "PL-001-核心架构")）。
//!
//! ## 为什么第零天就位
//!
//! 权限是**静默失效**的——违反时没有测试会失败（J3）。事后补，代价是重写所有
//! 历史事件（[plan/01 §7](../../../../docs/004-plan/001-核心架构.md "PL-001-核心架构")）。所以 S3 一阶
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
//! [plan/04 §3](../../../../docs/004-plan/004-工程落地.md "PL-004-工程落地") 第 7 步）；查询对未知主体
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
//! ## 生产接线（[04 §3.1 批⑥](../../../../docs/004-plan/004-工程落地.md "PL-004-工程落地")）
//!
//! 矩阵**构造**在 [`crate::symbio_core::authz`]（本机主体清单，与本域同住一棵模块树），
//! **判定**在 `plugins/session` 两处：
//!
//! | 侧 | 调用点 | 判什么 |
//! |---|---|---|
//! | 写侧（发言） | 运行器落**收束格**前（`turn_runner::closure_granted`，`ClosureKind::Speech`） | [`can_reply`]（首条 → `ReplyFirst`，后续 → `ReplyAppend`） |
//! | 写侧（产物） | 运行器落 `artifact.added` 前（`turn_runner::closure_granted`，`ClosureKind::Artifact`） | [`can_produce_artifact`]（`ProduceArtifact`） |
//! | 读侧 | `stats` 按读方声明判（属主 = `SESSION_OWNER`，部署事实） | [`can_see`]（`thread_private` 缺省，C10）⇒ 非属主读数为空 |
//!
//! 两道写侧闸判的是**两件事**（[ADR-067](../../../../docs/010-adr/067.md "ADR-067")：收束格按种类判）：
//! 发言收束问 `reply.*`、产物收束问 `produce.artifact`。没有发言权的主体（对等主体
//! `agent:peer-b` 只持 `produce.artifact`）因此仍能以**产物**收束一轮——「没有说话
//! 资格就不能留下任何事实」正是那次裁决拆掉的耦合。两道闸的谓词都只定义在**本域**，
//! 调用点不复述（口径只活在 core）。
//!
//! 表为什么与机制同住 core：它是**架构元素**——「谁持有什么能力」与本域的 7 项闭集
//! 是一张表的两半，分家的代价是「表授予 A、闸门判 B」这种错位**没有任何东西会变红**。
//! 表仍然**不进任何插件**（E-009：插件互不可见，任一插件持有它都会逼别的插件跨插件
//! 引用），也**不进对外公开面**（`pub(crate)` 开给全 crate）——表里写的是部署事实，
//! 不是契约。
//!
//! ## 依赖纪律（[plan/05 §3.1](../../../../docs/004-plan/005-模块架构.md "PL-005-模块架构") ⑥ 行）
//!
//! 只依赖类型定义 + ④ store 的**读面**（事件切片），只读写事件——不持有
//! ①②③⑤ 的任何句柄。

/// 权限能力（[plan/01 §7](../../../../docs/004-plan/001-核心架构.md "PL-001-核心架构")：**7 个封顶**）。
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
    /// **轮次位置 → 收束能力**的唯一映射（首条 → `ReplyFirst`，后续 → `ReplyAppend`）。
    ///
    /// 两处读者共用它、各自不抄清单：授权表谓词 [`PermissionMatrix::can_reply`]
    /// 与规格谓词 [`crate::symbio_core::ActorSpec::can_land_speech`]（T-082 批①：
    /// 写侧闸判改问规格身上那份能力集）。映射若分两份，改一处即静默错位。
    pub(crate) fn reply_capability(first: bool) -> Capability {
        if first {
            Capability::ReplyFirst
        } else {
            Capability::ReplyAppend
        }
    }

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

    /// 能力名（授权表的**书写形态**）——[`Self::from_name`] 的逆向。
    ///
    /// 读进来与写出去共用同一份映射：两处各写一遍字符串，迟早漂移成「读作 A、
    /// 写作 B」——而两边都是合法能力名，没有任何东西会为此变红。
    pub(crate) fn name(&self) -> &'static str {
        match self {
            Capability::JudgeIntent => "judge.intent",
            Capability::ReplyFirst => "reply.first",
            Capability::ReplyAppend => "reply.append",
            Capability::DefineWork => "define.work",
            Capability::ProduceArtifact => "produce.artifact",
            Capability::AssertVerification => "assert.verification",
            Capability::AssignWork => "assign.work",
        }
    }
}

/// 可见域（[plan/01 §8](../../../../docs/004-plan/001-核心架构.md "PL-001-核心架构") 参数表：3 个取值）。
///
/// **C10**（[plan/04 §4](../../../../docs/004-plan/004-工程落地.md "PL-004-工程落地")）：默认
/// `thread_private`——默认隔离是**缺省值**，不是需要每个主体记得配置的东西。
///
/// ## 字符串形态：内容侧可见域的载荷书写（[T-094](../../../../docs/009-task/094-个人认知可见域产地批.md "T-094-个人认知可见域产地批")）
///
/// [`Self::name`] / [`Self::from_name`] 是**载荷里的书写形态**（`memory.encoded`
/// 的 `vis_scope` 字段）：判定走枚举，落盘走字符串，两套形态共用同一份映射——
/// 各写一遍字符串迟早漂移成「写的是 shared、读的是 public」，而两边都是合法值，
/// 没有任何东西会为此变红。缺省（载荷缺这一字段）恒为 [`VisScope::ThreadPrivate`]：
/// 存量事件没有这个字段，把缺席读成别的值等于给历史事实改判。
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

impl VisScope {
    /// 书写形态（`from_name` 的逆向）——判据只在这里出现一次。
    pub fn name(&self) -> &'static str {
        match self {
            VisScope::ThreadPrivate => "thread_private",
            VisScope::Shared => "shared",
            VisScope::Public => "public",
        }
    }

    /// 书写形态 → 枚举。认不出的值 = [`VisScope::ThreadPrivate`]（缺省口径，
    /// 与载荷缺这一字段同义）——**不**把拼写错误静默读成更宽的可见域，
    /// 那等于把笔误变成一次越权读取。
    pub fn from_name(name: &str) -> VisScope {
        match name {
            "shared" => VisScope::Shared,
            "public" => VisScope::Public,
            _ => VisScope::ThreadPrivate,
        }
    }

    /// 两个可见域里**更宽**的那个（`thread_private` < `shared` < `public`）。
    ///
    /// 唯一的合并判据：读侧要就一批内容回答「这份整体给不给人看」时用它
    /// （T-094 ② 的整列闸）——各调用点自己排大小就会漂移成「这边 public 赢、
    /// 那边 shared 赢」，而两种答案都是合法取值，没有任何东西会变红。
    pub fn widen(self, other: VisScope) -> VisScope {
        use VisScope::{Public, Shared, ThreadPrivate};
        match (self, other) {
            (Public, _) | (_, Public) => Public,
            (Shared, _) | (_, Shared) => Shared,
            _ => ThreadPrivate,
        }
    }
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
    /// 质量闸门等于没有（S7 第 17 步，[roadmap/S03 §5](../../../../docs/009-task/123-多步任务与返工.md "T-123-多步任务与返工")）。
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
    /// [plan/04 §3](../../../../docs/004-plan/004-工程落地.md "PL-004-工程落地") 第 7 步）。
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
            // 高风险组合（S9，[roadmap/S09 §5](../../../../docs/009-task/129-外部执行与熔断.md "T-129-外部执行与熔断")
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

    /// 写侧：主体持有的能力**名**（授权表的书写形态）。
    ///
    /// 为什么查询面也要名字形式：[`Capability`] 刻意不进 core 根平铺（见模块头
    /// 「同名辨析」）⇒ **插件侧拿不到那个枚举**，只能拿名字。而 `ActorSpec` 的
    /// `capabilities` 本来就是**数据**（`Vec<String>`）——它的唯一生产写方从授权表
    /// 行取数，取出来的必须是同一套拼写。名字的对错由**构造期**兜住
    /// （[`Self::from_names`] 认不出即拒绝整体构造），读侧照单全收于是不会变成
    /// 「拼写错误被读成拒绝」的第二处判定。
    pub fn grant_names_of(&self, principal: &str) -> Vec<String> {
        self.grants_of(principal)
            .iter()
            .map(|c| c.name().to_string())
            .collect()
    }

    /// 写侧判定：主体是否可以权威地做 `cap`。**fail-closed**。
    pub fn can_write(&self, principal: &str, cap: Capability) -> bool {
        self.grants_of(principal).contains(&cap)
    }

    /// 写侧判定（**按能力名**）：`name` 是授权表的书写形态（[`Self::from_names`] 按
    /// 7 项闭集校验），与构造面同一套字符串。
    ///
    /// 为什么查询侧也要一个按名入口：[`Capability`] 刻意不进根平铺（见模块头
    /// 「同名辨析」），core 的模块门也不对外开 ⇒ **core 外拿不到那个枚举**，
    /// 只能写名字。名字的对错由构造期兜住，查询侧照单全收于是不会变成
    /// 「拼写错误被读成拒绝」的第二处判定——`from_name` 认不出即 `None`
    /// ⇒ `false`，fail-closed 的方向与 [`Self::can_write`] 完全一致。
    pub fn can_write_name(&self, principal: &str, name: &str) -> bool {
        Capability::from_name(name).is_some_and(|cap| self.can_write(principal, cap))
    }

    /// 写侧判定（收束闸）：主体能否**收束一轮发言**——运行器落收束格前的判据。
    ///
    /// `first` = 该轮尚无收束格：首条 → [`Capability::ReplyFirst`]，后续（重试 /
    /// 追加）→ [`Capability::ReplyAppend`]。「轮次位置 → 能力」的映射只在
    /// [`Capability::reply_capability`] 一处定义，调用点不复述（口径只活在 core）。
    /// 未持有 ⇒ 拒绝入格（fail-closed）。
    pub fn can_reply(&self, principal: &str, first: bool) -> bool {
        self.can_write(principal, Capability::reply_capability(first))
    }

    /// 写侧判定（**产物闸**）：主体能否**产生产物**——运行器落 `artifact.added`
    /// 前的判据（`turn_runner::closure_granted` 的 `ClosureKind::Artifact` 臂）。
    ///
    /// 与 [`Self::can_reply`] 并列而**不是**它的一个分支（[ADR-067](../../../../docs/010-adr/067.md "ADR-067")）：
    /// 收束有两条路，谓词各问自己那一项能力。对等主体（`agent:peer-b`）只持
    /// `produce.artifact`，发言闸对它恒拒而产物闸恒放——它因此能收束为**一件
    /// 产物**，而不是每一轮都留下一条 C4 未收束。
    ///
    /// 「产物收束 ⇒ 该轮已收束」由 C4 不变量同批认可（见该函数文档）：产物格
    /// 带 `produced_by` 指向本轮开口格，它与 final / fallback 一样是「这一轮
    /// 有话要说」的完成形态。
    pub fn can_produce_artifact(&self, principal: &str) -> bool {
        self.can_write(principal, Capability::ProduceArtifact)
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

/// 授权变更（淘汰回路的一格，[roadmap/S13 §3](../../../../docs/009-task/133-多智能体社会.md "T-133-多智能体社会")）：
/// 对某主体某项能力的一次**收回**或**恢复**。
///
/// ## 为什么是数据而不是状态
///
/// S13 §5 的强制项「淘汰必须可申诉」与 §6 断言 3「淘汰 = 一条 grants 变更事件，可被
/// 审计、可被撤销」要求的正是这个形状：「谁被收回了什么」若只活在内存布尔里，
/// 审计无从谈起（没有流水），撤销也无从谈起（没有逆操作）。变更经事件通道落盘，
/// 撤销就是往同一条通道里再写一条 `Restore`——last-wins 折叠出当前状态，
/// 全量流水就是审计面（[`GrantsOverlay::changes`]）。
///
/// 能力用**名**（授权表的书写形态）不用枚举：变更的生产方在 `actors`（裁决侧），
/// 消费方在插件侧（闸门）——`Capability` 刻意不进根平铺（见模块头「同名辨析」），
/// 两边都只够得着名字。名字的对错在**判定点**按 [`Capability`] 闭集收口
/// （[`GrantsOverlay::is_revoked`]）：带认不出能力名的**收回**项不静默落空——
/// 它按该主体整体已收回判（一次笔误的收回不得悄悄消失）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrantsChange {
    /// 收回：`verdict.asserted`（`change = "revoke"`）折叠出的那一格。
    Revoke {
        principal: String,
        capability: String,
    },
    /// 恢复：申诉成立 / 人工复位的逆操作（`change = "restore"`）。
    Restore {
        principal: String,
        capability: String,
    },
}

impl GrantsChange {
    /// 变更指向的主体。
    pub fn principal(&self) -> &str {
        match self {
            GrantsChange::Revoke { principal, .. } => principal,
            GrantsChange::Restore { principal, .. } => principal,
        }
    }

    /// 变更指向的能力名（书写形态）。
    pub fn capability(&self) -> &str {
        match self {
            GrantsChange::Revoke { capability, .. } => capability,
            GrantsChange::Restore { capability, .. } => capability,
        }
    }

    /// 是否收回（`is_revoked` 的判定半边）。
    pub fn is_revoke(&self) -> bool {
        matches!(self, GrantsChange::Revoke { .. })
    }
}

/// 部署级授权覆盖层：把事件流里历次 [`GrantsChange`] 折叠成「现在谁被收回了什么」。
///
/// ## 为什么需要它（而不是直接改表）
///
/// 授权表（`authz::ROWS`）是**常量**——宿主装配期定稿，运行期不改。淘汰却是运行期
/// 事实：声誉掉到阈值以下的主体要被收回 `assign.work`（S13 §2「淘汰」行）。两条路
/// 都不通：把表改成可变的 ⇒ 「表授予 A、闸门判 B」的漂移面回来了（表不再是单一
/// 权威）；不收回 ⇒ 淘汰这条断言在生产里没有对应物（§6 断言 3 未点亮）。
///
/// 覆盖层于是住在表**旁边**而不是表**里面**：表回答「这个主体登记过什么」，
/// 覆盖层回答「其中有什么现在被收回了」。闸门的判定 = 表谓词 **且非** 覆盖层撤销
/// ——两个问题各有一个答案，合成才是「现在准不准」。
///
/// ## 折叠口径：last-wins
///
/// 同一 `(principal, capability)` 的多次变更，**最后一次**生效——恢复之后再收回
/// 就又是收回，反之亦然。判据从流水尾部往前找，找到的第一条就是当前状态。
/// 认得出能力名的变更才进这条折叠；认不出名字的收回按主体整体生效
/// （[`Self::is_revoked`]），同名 `Restore` 也不解除。
///
/// ## 审计面在事件流里，不在本类型上
///
/// 「可审计」（S13 §6 断言 3）的全部含义是**流水可查**：`verdict.asserted` 事件
/// 就在 Log 里（经 `EliminationDecider::verdict_grants_changes` 读），本类型只做
/// 折叠与判定。曾设想过 `changes()` / `revoked()` 两个便利读数，但它们**没有生产
/// 消费方**——按本项目纪律（「接线了却永不生效」正是要拦的静默失效）下线：
/// 需要审计时读事件流，那是不可变的那一份。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GrantsOverlay {
    changes: Vec<GrantsChange>,
}

impl GrantsOverlay {
    /// 由变更流水构造（顺序即时间序：fold 判据从尾部往前找）。
    pub fn fold(changes: impl IntoIterator<Item = GrantsChange>) -> Self {
        GrantsOverlay {
            changes: changes.into_iter().collect(),
        }
    }

    /// 该主体的该能力**现在是否被收回**。
    ///
    /// 判据分三段，前两段是闭集 fail-closed（[ADR-099](../../../../docs/010-adr/099.md "ADR-099")）：
    ///
    /// - **查询名认不出**（不在 [`Capability`] 7 项闭集内）⇒ 告警并按**已收回**判——
    ///   「现在准不准」无从查起时宁红不漏，笔误的查询名不得变成一次静默放行；
    /// - **该主体的流水里有认不出能力名的收回项** ⇒ 对该主体的**任何**能力查询都按
    ///   已收回判：一次笔误的收回不得静默落空。这不是「某一格被收回」而是「这次
    ///   收回落不到任何一格」，故按主体整体生效，同名 `Restore` 也不解除
    ///   （代价：一次载荷损坏会让该主体暂时什么都做不了，但不会让一次本该生效的
    ///   收回消失）；
    /// - 其余按 **last-wins**：同一 `(principal, capability)` 从流水尾部往前找，
    ///   第一条命中就是当前状态。
    ///
    /// 与 [`PermissionMatrix`] 的表谓词合成才是「现在准不准」：表答「登记过什么」，
    /// 本判定答「其中什么现在被收回了」。
    pub fn is_revoked(&self, principal: &str, capability: &str) -> bool {
        if Capability::from_name(capability).is_none() {
            crate::plugin_warn!(
                "governance",
                "能力名 `{capability}` 不在 7 项闭集内（主体 {principal}），按已收回判"
            );
            return true;
        }
        if self.changes.iter().any(|c| {
            c.is_revoke()
                && c.principal() == principal
                && Capability::from_name(c.capability()).is_none()
        }) {
            crate::plugin_warn!(
                "governance",
                "主体 {principal} 的变更流水里有能力名认不出的收回项，任何能力查询按已收回判"
            );
            return true;
        }
        self.changes
            .iter()
            .rev()
            .find(|c| c.principal() == principal && c.capability() == capability)
            .is_some_and(|c| c.is_revoke())
    }
}

#[cfg(test)]
#[path = "mod.test.rs"]
mod tests;
