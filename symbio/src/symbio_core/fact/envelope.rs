//! 事实信封 —— [`Fact`] 结构体、身份、溯源、派生源 trait。
//!
//! ## 只读的构造形态（吸取投影纯度那一课）
//!
//! [`Fact`] 的字段**全部 `pub`**（它是数据不是不变量），但**没有任何写入口**——
//! 它没有 `append`、没有 `Store`、没有 `&mut self` 方法。「写入仍走各自原有路径」
//! 这条边界不靠约定，靠**类型上不提供写方法**。
//!
//! 为什么不像 v2 的 `Projection` 那样做成私有字段 + `new` 门？
//! 因为两者的**风险不同**：`Projection` 的风险是"带副作用"，所以要拦构造；
//! `Fact` 的风险是"变成第二份真相"，所以要拦**写**。拦写的方式是**不提供写方法**
//! （比私有字段更彻底——私有字段还能在模块内被改）。
//!
//! ## 派生源：[`FactSource`]
//!
//! 谁拥有数据，谁负责把数据派生为 `Fact`。派生必须满足两条：
//!
//! 1. **纯**：同一份源数据 → 同一串 `Fact`（`seq` 顺序确定）。因此可双跑比对。
//! 2. **有序**：`seq` 在返回的切片内**严格递增**（它是回放与溯源的基础）。
//!
//! ## 溯源（v2 的 I2）
//!
//! [`Fact::caused_by`] 指向**更早的 seq**。因为 `seq` 严格递增，
//! 溯源边只能从高 `seq` 指向低 `seq`，派生图**天然无环**——不需要额外的无环检查。
//! 断言类事实（[`FactKind::is_assertion`]）必须带溯源，由断言 A2 强制。

use super::kind::FactKind;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// 「无前驱」哨兵 —— `caused_by = None` 的线上表示。
///
/// 用 `0` 而不是 `null`：`seq` 从 **1** 起（`0` 保留给"不存在"），
/// 于是"指向 0"与"无前驱"在数字上不可混淆。
pub const FACT_NONE_SEQ: u64 = 0;

/// 事实主体身份 —— v2 的 `Principal`。
///
/// 是**字符串数据**（无限增长），不是枚举。现行 `session_id` / `agent_id` /
/// 插件目录名都是它的合法取值。
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FactPrincipal(pub String);

impl FactPrincipal {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for FactPrincipal {
    fn from(s: &str) -> Self {
        Self(s.to_string())
    }
}

/// 统一事实信封 —— 全系统可观测事实的**只读**表示。
///
/// 字段全 `pub`，但**无写入口**：写入仍走各自原有路径（见模块文档）。
/// 使用方只能读，不能造——`Fact` 的生产者是 [`FactSource`]。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fact {
    /// 全局单调序号。**跨会话、跨插件**唯一。
    ///
    /// 从 **1** 起（`0` 保留给 [`FACT_NONE_SEQ`]）。同一份源数据双跑必须得到同一串 `seq`。
    pub seq: u64,
    /// 事实类型。
    pub kind: FactKind,
    /// 谁产生的。
    pub principal: FactPrincipal,
    /// 溯源：指向**更早的 `seq`**（见模块文档）。
    ///
    /// 断言类事实（[`FactKind::is_assertion`]）必须非空——这是不变量 I2。
    pub caused_by: Option<u64>,
    /// 发生时刻（Unix 毫秒）。
    pub at_ms: i64,
    /// 载荷：**沿用既有 schema**，不新建第二套结构。
    ///
    /// 会话事实装 `ChatMessage` 的 JSON 形状，资源事实装 VDFS 变更形状——
    /// 具体形状由 [`FactSource`] 实现方决定并在其文档里写清。
    pub payload: serde_json::Value,
}

impl Fact {
    /// 是否满足"无溯源不声明"（I2）：断言类事实必须有 `caused_by`。
    ///
    /// 这是断言 A2 的**可复用判据**——它住在事实定义旁边，
    /// 使"违规判定"与"事实定义"不会分叉。
    pub fn has_provenance(&self) -> bool {
        !self.kind.is_assertion() || self.caused_by.is_some()
    }

    /// 溯源是否指向**更早**的 seq（派生图无环的充分条件）。
    ///
    /// `caused_by == Some(x)` 时要求 `x < self.seq`；`None` 恒真；
    /// 指向 [`FACT_NONE_SEQ`] 视为"无前驱"（合法）。
    pub fn provenance_points_back(&self) -> bool {
        match self.caused_by {
            None => true,
            Some(FACT_NONE_SEQ) => true,
            Some(x) => x < self.seq,
        }
    }
}

/// 事实派生错误 —— 派生是纯函数，"失败"只可能是源数据不可解析。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FactError {
    /// 源数据不可解析（如消息 JSON 结构损坏）
    Malformed(String),
}

impl std::fmt::Display for FactError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FactError::Malformed(m) => write!(f, "事实派生失败：源数据不可解析（{m}）"),
        }
    }
}

impl std::error::Error for FactError {}

/// 事实派生源 —— 谁拥有数据，谁实现它，把数据派生为 [`Fact`] 序列。
///
/// ## 契约
///
/// - **纯**：同一份源数据 → 同一串 `Fact`（可双跑比对）；
/// - **有序**：返回切片的 `seq` **严格递增**；
/// - **溯源向后**：每个 `Fact` 的 `caused_by`（若非空）指向更早的 `seq`。
///
/// ## 为什么是 trait
///
/// 实现方分布在**不同插件**（会话事实由 `session` 派生、资源事实由 `vdfs` 派生），
/// 而消费方（检索者、巩固者等新能力）需要**按统一形状**取用。
/// 插件之间不可相互引用，因此这条契约只能住在 core——这与
/// [`VdfsProvider`](crate::symbio_core::vdfs::VdfsProvider)、
/// [`Capability`](crate::symbio_core::capability::Capability) 同型。
///
/// ## 平凡值（J2）
///
/// **不实现它 = 不贡献事实**，系统其余部分照常运行。这是本域的平凡值：
/// 未接入 `FactSource` 的插件不会因此报错。
#[async_trait]
pub trait FactSource: Send + Sync {
    /// 派生**[from, head)** 区间的事实，`seq` 严格递增。
    ///
    /// `from = 0` 表示"从头"。实现方可以只支持全量派生（`from` 被忽略），
    /// 但必须保证返回结果的 `seq` 区间语义正确——增量是优化，不是契约。
    async fn facts(&self, from: u64) -> Result<Vec<Fact>, FactError>;

    /// 当前已知的最大 `seq`（`0` = 空）。
    fn head(&self) -> u64;
}
