mod collect;
mod configurable;
mod error;
mod model;
mod mount;
mod option;
mod prompt;
mod row;
mod tool_name;

// 三条收集通道键与错误桶键住本域（键与值同住，ADR-095 根因一）。**只导实例、
// 不导 `…Key` 类型**——实例是 `ctx` 上的读写凭据（对外协议面），类型名没有任何
// 域外消费方（C-003），`pub struct` 留在子模块里由本域自用。`CapabilityError`
// 同理不出本域：读侧经 `capability_take_errors` 的返回类型推断拿到它，
// 无需实名（字段 `plugin` / `message` 直接可读）。
pub use collect::{tool_metas, CAPABILITY_VISITOR, CONFIGURABLE_VISITOR, OPTION_VISITOR};
pub use error::{
    capability_init_error_bucket, capability_report_error, capability_take_errors,
    CAPABILITY_ERRORS,
};

// 域内子模块私有，公开面在此显式重导出
//
// 注：全表共用的内存收集器（`DefaultToolVisitor`）**不在本域**——
// 它是契约的默认实现，写入者是全体插件，故住 `crate::providers::collectors`。
pub use configurable::{
    capability_announce_configurable, capability_configurables, capability_entry_of,
};
pub use model::{capability_model_active, capability_model_resolve};
pub use mount::{capability_mount, capability_mounts, capability_vdfs_root};
pub use option::capability_option_fields;
pub use prompt::capability_prompts;
pub use row::{CapabilityRow, CapabilityRowKind};
pub use tool_name::{capability_resolve, capability_to_wire};

use crate::symbio_core::{
    ExecEnv, PluginError, PluginInvokeRequest, PluginInvokeRequestExt, PluginInvokeResponse,
    PluginPayload,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

/// 能力分类（`CapabilityMeta.category` 的唯一分类字段）
///
/// 设计原则：
/// - `CapabilityCategory` 是**机制化的语义标签**，与具体语言无关
/// - `CapabilityMeta.category: Option<CapabilityCategory>` 是唯一分类字段
/// - 分类只能用本枚举的变体表达（不接受字符串字面量），编译器强制这一点
/// - **不携带展示文案**：本枚举只承载语义，本地化与呈现由消费方自己决定
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityCategory {
    /// 智能体协作：chat / handoff
    Chat,
    /// 核心能力：能力自身管理（UnifiedCapabilityTool）
    Core,
    /// 技能调用：外部 skill / sub-skill
    Skill,
    /// 文件操作：read / write / edit / glob / search
    FileOperation,
    /// 网络搜索：web_search / web_fetch / http_request
    Network,
    /// 系统操作：shell
    SystemOperation,
    /// MCP 工具：来自外部 Model Context Protocol server
    Mcp,
    /// 资源管理：VDFS 虚拟文件系统操作（read / write / list / tree / stat）
    Resource,
    /// 未分类：兜底
    #[default]
    Other,
}

/// 大语言模型工具定义
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CapabilityMeta {
    /// 工具名称
    pub name: String,
    /// 工具描述
    pub description: String,
    /// 参数 Schema (JSON Schema)
    #[serde(rename = "parameters")]
    pub input_schema: Value,
    /// 能力分类（唯一分类字段，类型为枚举）
    ///
    /// - `Some(枚举)`：渲染层按 `ctx.get("lang")` 选本地化文案
    /// - `None`：兜底为 `Other`（展示"其他"）
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub category: Option<CapabilityCategory>,
    /// 使用示例列表
    #[serde(skip_serializing_if = "Option::is_none")]
    pub examples: Option<Vec<String>>,
    /// 工具风险等级（`None` = 默认 `Medium`，由 [`effective_risk`](Self::effective_risk) 兜底）
    ///
    /// 机制说明：工具在 `CapabilityMeta` 中**自声明**风险等级，审批闸门
    /// （`local::policy`）读这个字段判定，**不硬编码工具名**——工具增删不改闸门。
    /// 取值只能是枚举变体（不接受字符串字面量），编译器强制这一点。
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub risk: Option<CapabilityRiskLevel>,
    /// 本工具是否提供 `assign.work`（派活）——**淘汰闸的管辖对象**（S13 §3，T-073）。
    ///
    /// 机制说明：与 [`risk`](Self::risk) 同一条纪律——**工具自声明**，闸门读这个
    /// 字段判定，**不硬编码工具名**（T-158）。`assign.work` 被淘汰回路收回时，
    /// 执行器据本字段把调用判为淘汰拒止；未声明的工具不在淘汰闸管辖内。作用域
    /// 投影（子智能体的改名包装）只换名字、原样透传本字段——闸门跟随**工具**，
    /// 不跟随它在哪个作用域里叫什么。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub provides_assign_work: bool,
    /// 本工具是否**立约**（跨主体承诺）——承诺出参的记账对象（S08 §3）。
    ///
    /// 机制说明：同 [`provides_assign_work`](Self::provides_assign_work) 的自声明
    /// 纪律（T-158）。承诺记账（`commitment.*` 格）只为声明了本位的工具记；没跑
    /// 到的一样记违约——「约」本身已经存在。未声明 ⇒ 不记：本主体自办的调用是
    /// 执行结果，不是承诺。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub records_delegation_promise: bool,
    /// 本工具是否写**任务表**——任务表写侧资格闸与任务声明记账的管辖对象
    /// （S03 §3，T-107）。
    ///
    /// 机制说明：写侧资格（`define.work` / `assert.verification` 二居其一）与任务
    /// 声明记账都只为声明了本位的工具生效（T-158）。线上名 `<挂载点>/<短名>` 取
    /// 末段这个**匹配口径**不变，变的是「谁是管辖对象」——从执行器里的字面量
    /// 改成工具自己的声明。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub owns_task_table: bool,
}

impl CapabilityMeta {
    /// 构造带 category 分类的元数据
    pub fn with_category(mut self, kind: CapabilityCategory) -> Self {
        self.category = Some(kind);
        self
    }

    /// 构造带风险等级的元数据
    pub fn with_risk(mut self, risk: CapabilityRiskLevel) -> Self {
        self.risk = Some(risk);
        self
    }

    /// 读取生效的风险等级（未声明视为 `Medium`）
    pub fn effective_risk(&self) -> CapabilityRiskLevel {
        self.risk.unwrap_or_default()
    }
}

/// 工具风险等级（`CapabilityMeta.risk` 的唯一取值）
///
/// **默认档是 `Medium`，不是 `Low`**：未声明风险的工具走 [`Default`]，而
/// [`CapabilityMeta::effective_risk`] 用它兜底。判据方向是「宁可多问一次」——
/// 未声明即视为可能改状态（Medium），阈值调到 low 的会话仍会为它要一次审批；
/// 若默认 Low，同一个未声明的工具在任何阈值下都免审批，声明就失去了约束力。
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityRiskLevel {
    /// 低风险：只读、不触碰资源（如问答、搜索、读取）
    Low,
    /// 中风险：写入、修改、有副作用但可回滚
    ///
    /// **未声明风险的工具按此档兜底**（见类型文档）。
    #[default]
    Medium,
    /// 高风险：破坏性、不可逆、系统级操作
    High,
}

impl CapabilityRiskLevel {
    /// 从字面量解析 —— **本档位字符串的唯一解析点**。
    ///
    /// 未识别的值按 [`Self::default`] 兜底（`Medium`），与「未声明风险的工具」同一条
    /// 判据：宁可多问一次，不因为写错一个词就静默放宽。
    ///
    /// 为什么要有这个方法：会话阈值（`ctx[RISK_LEVEL]`）此前在两处**各写一份**
    /// 五字 `match`（审批闸门 `providers/collectors/approval.rs` 与 shell 的
    /// 参数级风险判定）。加一档 `Critical` 要改两处，漏一处就静默降级成 `Medium`——
    /// 那是**放宽安全约束**的静默失效，不是缺个功能。现在两处都问这里。
    pub fn from_name(name: &str) -> Self {
        match name {
            "low" => Self::Low,
            "high" => Self::High,
            // 含 "medium" 与一切未识别的值。
            _ => Self::Medium,
        }
    }
}

#[async_trait]
pub trait Capability: Send + Sync + 'static {
    fn meta(&self) -> CapabilityMeta;

    fn name(&self) -> String {
        self.meta().name
    }

    /// 本次调用的**生效风险等级**（审批闸门读它）。
    ///
    /// 默认实现 = [`CapabilityMeta::effective_risk`]，即「工具自声明的静态档」。
    /// 绝大多数工具到此为止；**风险随参数变化**的工具覆写它——shell 是唯一
    /// 已知的例子：`rm -rf ./build` 与 `git status` 同属一个工具，风险却差两档。
    ///
    /// ## 为什么不写成「闸门按工具名特判」
    ///
    /// 闸门里出现 `if name == "cmd"` 这类判定，等于把「有哪些工具」这份清单
    /// 复制到审批层：工具改名/新增/改名都要回来改闸门，而漏改的表现是**静默放行**
    /// （新工具落到默认档，免审批）。让工具自己回答「我这次有多危险」，闸门只负责
    /// 拿这个数去比阈值——判定与清单就都留在工具这一侧了。
    fn risk_for(&self, args: &Value) -> CapabilityRiskLevel {
        let _ = args;
        self.meta().effective_risk()
    }

    /// 执行能力调用。
    ///
    /// 三个入参各有一义，不再混在同一个键值袋里：
    /// - `args`：**模型给的参数**（工具调用参数 / 直接调用的 payload）；
    /// - `env`：**执行期环境**（出口 + 中止）——与 `ModelProvider::execute_turn`
    ///   共用同一类型，见 [`ExecEnv`]；
    /// - `ctx`：**请求信封**——只有需要转发（`agent_run` 起子会话经 VDFS 地址写入、
    ///   MCP 网关、VDFS 挂载解析）或读会话上下文（`WORKDIR` / `RISK_LEVEL`…）的
    ///   工具才碰它。
    ///
    /// 返回**工具结果本身**（任意 JSON），不再经 `PluginPayload` 那层多态载荷：
    /// 工具从来只用 `Data` 一个变体，其余三个（`Empty` / `Native` / `Session`）
    /// 是路由层的形态，与工具无关。信封 ↔ 结果的换算收口在 [`capability_invoke`]。
    async fn execute(
        &self,
        args: Value,
        env: &ExecEnv,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> Result<Value, PluginError>;
}

/// 把请求信封拆成 `(args, env, ctx)` 调用能力，再把结果装回信封。
///
/// ## 这是唯一「拆信封」的地方
///
/// 所有分发路径（按名执行的两步：`get` + 本函数 / `Plugin::route` 的工具分支 /
/// 装饰器）都必须经这里，否则「工具怎么写」与「信封长什么样」会重新耦合——
/// 而耦合的表现就是每个工具各写一份 `ctx.payload::<Value>().unwrap_or(...)`
/// 与各读一次 `EVENT_SINK` / `ABORT_SIGNAL`。
///
/// 参数缺席（信封里没有 payload）⇒ `Value::Null`：与收口前各工具自己的
/// `unwrap_or(Value::Null)` 口径一致，由工具自己给出「缺少必填参数」的报错。
pub async fn capability_invoke(
    cap: &dyn Capability,
    ctx: Arc<dyn PluginInvokeRequest>,
) -> PluginInvokeResponse<PluginPayload> {
    let args = ctx.payload::<Value>().unwrap_or(Value::Null);
    let env = ExecEnv::from_request(&*ctx);
    cap.execute(args, &env, ctx)
        .await
        .map(|value| PluginPayload::new(&value))
}

/// 工具结果里的 `failure_kind` 闭集 —— 「本轮结束于等待用户动作」的两种形态。
///
/// ## 为什么需要共享常量
///
/// 这些词是**工具（生产方）与编排层（消费方）之间的约定**，而两者分属不同插件
/// （`local` / `session`），互相不可见，只能经 `symbio_core` 共享。写成字面量会
/// 立刻分裂：编排层里已有一处硬编码判定
/// （`k == "error" || k == "needs_approval" || k == "needs_interaction"`），
/// 再加一个「等待用户」的 kind 就得记得同步改它——忘掉的表现是**交互模式下本批
/// 剩余工具照跑**，即用户本该逐个处理却收到一堆并发审批。
///
/// ## 为什么不是枚举
///
/// `failure_kind` 落在消息 `meta`（JSON）里，与 `error` 这类**信息性**取值同一个
/// 字段。它是给渲染层看的字符串，不是 Rust 侧的判别联合；强行枚举会把「信息性
/// 标记」升级成「必须穷举的状态」。真正的判据只有一条，见 [`failure_kind::is_pending`]。
pub mod failure_kind {
    /// 工具失败（信息性：错误结果回传 LLM 继续，不暂停会话）
    pub const ERROR: &str = "error";
    /// 需要用户**审批**（交互模式）——本轮收口为 `WaitingUserAction`
    pub const NEEDS_APPROVAL: &str = "needs_approval";
    /// 需要用户**回答**（`ask_user`）——本轮收口为 `WaitingUserAction`
    pub const NEEDS_INTERACTION: &str = "needs_interaction";
    /// 权限不足且无人可授权（自动模式）——**不**收口为等待，让 LLM 改走别的路
    pub const PERMISSION_DENIED: &str = "permission_denied";
    /// 工具当前不可用（自动模式）——同上
    pub const TOOL_UNAVAILABLE: &str = "tool_unavailable";

    /// 是否表示「本轮结束于等待用户动作」。
    ///
    /// 编排层凭这**一个**判据决定：构造 user_prompt 节点 + 把父 ToolCall 收口为
    /// `WaitingUserAction`。工具侧只声明意图（返回该 kind + `prompt` 载荷），
    /// 不构造节点——节点身份（`result_msg_id`）归编排层。
    pub fn is_pending(kind: &str) -> bool {
        matches!(kind, NEEDS_APPROVAL | NEEDS_INTERACTION)
    }
}

/// 收集口：**一张表，三个方法**（AR-025）。
///
/// 收敛前这里是十六个方法的形态：`register` / `register_batch` /
/// `list_capability` / `invoke` / `has_capability` 五个方法专服工具一族，
/// 模型 / 提示词 / 挂载又各有一套同形的方法。收敛后全部通道共用同一张表——
/// 写走 [`Self::put`]，按名读走 [`Self::get`]，按族读走 [`Self::list`]。
///
/// ## 一族一行，族别从载荷派生（落位：Tool → Model → 提示词 / 挂载 / 选项）
///
/// 每迁一族只加一个 payload 变体（`capability/row.rs`），**三个方法一个都不变形**——
/// 这正是十六方法糜烂的反面。族各自的专用词法（模型族的 `capability_model_*`）
/// 是表之上的自由函数，不回头往 trait 上挂方法。
///
/// ## 为什么 `invoke` 不在表上
///
/// 执行不是表的职责：表答「这一行是什么、怎么找到它」，
/// [`Capability::execute`] 答「跑起来是什么」。按名执行是 `get(name)` +
/// [`capability_invoke`](crate::symbio_core::capability_invoke) 两步——拆开之后，
/// 这三个方法在 4a–4e 各族（工具 / 模型 / 提示词 / 挂载 / 选项）全部复用：
/// 4b 的模型族落位时一个方法都没有变形（只加了行的形态）。
///
/// ## 顺序是表的一部分
///
/// [`Self::list`] 返回的顺序 = 注册顺序（工具族直接决定送给模型的 `tools` 数组
/// 顺序，见 ADR-057）。族内**不**按名字重排：重排等于每起一个进程换一次工具面
/// 前缀。策略行的先命中者胜排序（`(prio, name)`）住在 loop 插件的行表
/// （`loop_rows` / `decl::ring`），与表内注册序是两件事。
#[async_trait]
pub trait Registry: Send + Sync + 'static {
    /// 入表一行（**全族的唯一写入入口**）。
    ///
    /// 实现必须把每条进表的能力裹上审批闸
    /// （`providers/collectors/approval.rs`）：闸门住在收口点，`meta` /
    /// `risk_for` 原样透传，故 `get` / `list` 都不受影响，执行那一步才判定（T-138）。
    ///
    /// ## 单槽族的**唯一生效**也在这里维持
    ///
    /// 带生效位的行（[`CapabilityRow::active`]：模型族的当次生效者）入表时，实现必须
    /// 先把同族的旧生效行降位（[`CapabilityRow::demote`]）——单槽之所以是槽，就因为
    /// 它只装一个。行是 `Arc`，降位不是就地改位而是换一个副本入表：读者永远看见一份
    /// 完整的行，不会读到「两个生效者」的中间态。
    ///
    /// ## 表键是全表唯一的名字空间（族不参与键）
    ///
    /// 同名**跨族**也覆盖（后写者胜，与工具族的同名覆盖同一条 IndexMap 语义）。
    /// 名字撞车不是正常覆盖而是记账错误，故实现应留下一条告警再覆盖——
    /// 静默被顶掉的那一族不会以任何别的方式表现出来。
    async fn put(&self, row: CapabilityRow);

    /// 按名取一行（**取代了 `has_capability` 与 `invoke` 的查找**）。
    ///
    /// 名字是线上全名（工具族含挂载点前缀）。查不到返回 `None`，不猜、不回落
    /// ——调用方给诚实的 `Tool not found`。
    async fn get(&self, name: &str) -> Option<Arc<CapabilityRow>>;

    /// 按族列行（**取代了 `list_capability`**）。
    ///
    /// 顺序 = 注册顺序（见 trait 文档）。返回的是**行**而不是元数据：元数据经
    /// [`CapabilityRow::meta`] 现取（T-158 的三个声明位同样在 meta 上）。
    async fn list(&self, kind: CapabilityRowKind) -> Vec<Arc<CapabilityRow>>;

    // 模型族（4 方法，4b）与系统提示词族（2 方法，4c）已改走表：写入是在表的
    // 提示词行 / 模型行上 `put`，读取是 `capability_model_*` / `capability_prompts`
    // 这些自由函数。道理（写入语义与不变量如何从签名搬进结构与行词汇）各见
    // `capability/model.rs` 与 `capability/prompt.rs`。

    // 挂载族（5 方法，4d）也已改走表：挂载点与根都是行（根是带根位的保留行名行），
    // 写入是行上的 `put`，读取是 `capability_vdfs_root` / `capability_mounts` /
    // `capability_mount` 这些自由函数——理由（根为什么单槽、挂载清单是预留机制）
    // 见 `capability/mount.rs`。
}

#[cfg(test)]
mod tests;
