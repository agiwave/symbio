mod configurable;
mod error;
mod model;
mod option;
mod row;
mod tool_name;

pub use error::{
    capability_init_error_bucket, capability_report_error, capability_take_errors, CapabilityError,
};

// 域内子模块私有，公开面在此显式重导出
//
// 注：三个 `Default*Visitor`（能力 / 选项 / 可配置声明收集器的内存实现）**不在本域**——
// 它们是契约的默认实现，写入者是全体插件，故住 `crate::providers::collectors`。
pub use configurable::{
    capability_announce_configurable, capability_entry_of, ConfigurableVisitor,
};
pub use model::{capability_model_active, capability_model_resolve};
pub use option::OptionVisitor;
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

/// 工具上下文保留策略（会话机制化属性）
///
/// 工具在 `CapabilityMeta` 中声明"历史参数/结果在推理上下文中保留多少"，
/// 会话压缩层按声明通用处理，不对任何具体工具名做特殊化。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityToolContextRetention {
    /// 默认：完整保留，正常参与全局滑动窗口
    #[default]
    All,
    /// 仅保留最近 N 次调用的完整参数/结果，更早的调用在上下文中骨架化
    LastN(u32),
    /// 仅保留最近一次调用的完整参数/结果，更早的调用在上下文中骨架化。
    ///
    /// 适用于"每次写入全量状态、旧状态对后续推理无参考价值"的工具
    /// （如任务清单更新），可显著降低重复全量参数对上下文的占用。
    LastOnly,
}

impl CapabilityToolContextRetention {
    /// 每个工具应保留的完整调用次数（`All` → `u32::MAX` 表示不限制）
    pub fn keep_count(self) -> u32 {
        match self {
            CapabilityToolContextRetention::All => u32::MAX,
            CapabilityToolContextRetention::LastN(n) => n.max(1),
            CapabilityToolContextRetention::LastOnly => 1,
        }
    }
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
    /// 关键词列表（用于意图识别）
    #[serde(default)]
    pub keywords: Vec<String>,
    /// 能力分类（唯一分类字段，类型为枚举）
    ///
    /// - `Some(枚举)`：渲染层按 `ctx.get("lang")` 选本地化文案
    /// - `None`：兜底为 `Other`（展示"其他"）
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub category: Option<CapabilityCategory>,
    /// 使用示例列表
    #[serde(skip_serializing_if = "Option::is_none")]
    pub examples: Option<Vec<String>>,
    /// 工具上下文保留策略（`None` = 默认 `All`，完整参与滑动窗口）
    ///
    /// 机制说明：工具执行层把该策略 Stamp 到 ToolCall 消息 meta
    /// （`ctx_retention`），会话压缩层读取 meta 通用执行，会话侧
    /// 不感知任何具体工具名。
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub context_retention: Option<CapabilityToolContextRetention>,
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

    /// 构造带上下文保留策略的元数据
    pub fn with_context_retention(mut self, retention: CapabilityToolContextRetention) -> Self {
        self.context_retention = Some(retention);
        self
    }

    /// 构造带风险等级的元数据
    pub fn with_risk(mut self, risk: CapabilityRiskLevel) -> Self {
        self.risk = Some(risk);
        self
    }

    /// 读取生效的上下文保留策略（未声明视为 `All`）
    pub fn effective_context_retention(&self) -> CapabilityToolContextRetention {
        self.context_retention.unwrap_or_default()
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
/// 前缀。策略行的先命中者胜排序（`(prio, name)`）住在 session 的 `loop_registry`，
/// 与表内注册序是两件事。
#[async_trait]
pub trait CapabilityVisitor: Send + Sync + 'static {
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

    // 模型族（4 方法）已改走表：写入是 model 插件的逐行 `put`（生效位长在目录行上），
    // 读取是 `capability_model_active` / `capability_model_resolve` 两个自由函数——
    // 理由（「生效者必然是目录里的一员」如何从签名搬到结构）见 `capability/model.rs`。

    /// 注册系统提示词（按名称保序；同名覆盖）。
    ///
    /// 语义只有一条：**注册在这里的东西一定会送达模型**——全部条目按注册顺序
    /// 拼接，不做「取一个」的竞争。
    ///
    /// ## 为什么没有「基座 / 片段」两条通道
    ///
    /// `register_system_prompt`（人格基座，按优先级链取一个）与
    /// `register_system_prompt_segment`（机制级片段，全部追加）做的是**同一件事**
    /// ——往一个命名有序集合里塞一段文字——差异只在消费侧「取一个」还是「取全部」。
    ///
    /// 而「取一个」这个需求本身站不住：人格的**选择**发生在**注册期**（model 插件已按
    /// `PROVIDER_ID` 解析出唯一生效 provider，注册的就是当次该用的那一份），消费侧
    /// 再按 key 选一次是空动作。实测那两处「双键注册」（`provider_id` 与 `"default"`）
    /// 写的还是**同一份内容**。
    ///
    /// 于是合并成一个方法：一个注册入口、一个有序集合、一个 [`Self::list_system_prompts`]
    /// 出口。「谁在前谁在后」= 注册顺序 = 遍历顺序；要控制顺序就控制注册时机。
    ///
    /// ## 名字是覆盖键，不是分类
    ///
    /// 同名覆盖（`IndexMap` 语义）用于「同一段文字注册两次时取后者」，不承担任何
    /// 分类职责——不要用命名前缀去表达「这是人格」「这是记忆」。
    async fn register_system_prompt(&self, name: &str, prompt: String);

    /// 列出已注册的系统提示词（(name, prompt)，保注册顺序）
    async fn list_system_prompts(&self) -> Vec<(String, String)>;

    // ==================== VDFS provider（统一文件系统） ====================

    /// 注册一个 VDFS provider（**LLM 可控挂载机制**的注册面）。
    ///
    /// `name` 是**使用方选定的目录名**（组合根下的一级目录名，宿主机内唯一）——
    /// 约定用插件名（`PLUGIN_*` 常量），因为插件名天然唯一。provider 自身**不知道**
    /// 也不提供这个概念（见 `symbio_core::vdfs` 模块文档）。
    ///
    /// 与工具 / 模型服务 / 系统提示词**共用同一次 `traverse` 广播**：插件在
    /// `TRAVERSE_AVAILABLE_TOOLS` 分支里注册工具的同时顺带注册 provider，
    /// 会话链路（LLM 工具调用）与前端链路因此拿到同一份集合（含同一批目录名）。
    ///
    /// ## 定位：这是「暴露给 LLM 的资源」的可控入口
    ///
    /// 前端 / 系统链路经 [`crate::symbio_core::Plugin::get_vfs_provider`] 直接
    /// 查询（容器聚合），**不经过本通道**；本通道（含 [`Self::list_vdfs_providers`]
    /// / [`Self::get_vdfs_provider`]）是 **LLM 侧按名可控的挂载清单**——子智能体的
    /// 注册经 `SubAgentVisitor` 在这里加 `agent/<id>/` 前缀（作用域），将来给 LLM
    /// 按作用域 / 白名单裁剪可见资源时，消费方接在这组接口上。
    /// ⚠️ 当前 `vdfs_*` 工具取根走的是根单槽（[`Self::register_vdfs_root`]）；
    /// 本组接口暂无消费方**不是死代码**，是预留的可控机制——不要删除。
    ///
    /// 默认 no-op —— 不提供资源的实现方无需关心。
    async fn register_vdfs_provider(
        &self,
        _name: &str,
        _provider: Arc<dyn crate::symbio_core::VdfsProvider>,
    ) {
    }

    /// 列出已注册的 VDFS provider：`(目录名, 实现)`，按 `order` 升序稳定排序
    /// （语义与 [`Self::list_system_prompts`] 的 `(name, prompt)` 一致）
    async fn list_vdfs_providers(
        &self,
    ) -> Vec<(String, Arc<dyn crate::symbio_core::VdfsProvider>)> {
        Vec::new()
    }

    /// 按目录名查询 VDFS provider
    async fn get_vdfs_provider(
        &self,
        _name: &str,
    ) -> Option<Arc<dyn crate::symbio_core::VdfsProvider>> {
        None
    }

    /// 登记 `<根>` 的服务者（单槽，重复登记覆盖）。
    ///
    /// `<根>` 这棵子树由谁服务是**装配时的安排**：任何 provider 都可以被登记，
    /// 当前由 `composite` 登记（它恰好聚合了各子插件的子目录，见
    /// `plugins/composite/vdfs.rs`）。访问层（vdfs 插件）只取这个登记项再转发，
    /// 因此**拓扑知识不在访问层**——它只认「服务者 + 前缀分流」。
    ///
    /// 与工具 / 模型服务 / 系统提示词**共用同一次 `traverse` 广播**：容器在自己的
    /// `traverse` 分支里注册根，会话链路与前端链路因此拿到同一个根。
    ///
    /// **单槽归系统 Agent**：子智能体子树里也有容器（它会把自己登记成根），
    /// 其注册经 `plugins/agent/host/scope.rs::SubAgentVisitor` **丢弃**——否则子树
    /// 会劫持整个 `<根>`，系统侧全部挂载点瞬间消失。判据 = 该文件的单测 +
    /// e2e `t39`（细节归 `scope.rs` 的模块文档，此处只指路）。
    ///
    /// 默认 no-op —— 不是容器的实现方无需关心。
    async fn register_vdfs_root(&self, _provider: Arc<dyn crate::symbio_core::VdfsProvider>) {}

    /// 取 VDFS 根 provider；无容器注册时为 `None`（等价于「系统里没有资源」）
    async fn get_vdfs_root(&self) -> Option<Arc<dyn crate::symbio_core::VdfsProvider>> {
        None
    }
}

#[cfg(test)]
mod tests;
