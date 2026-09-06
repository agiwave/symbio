use crate::symbio_core::{InvokeRequest, InvokeResponse, PluginPayload};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;

/// 能力分类（v17：单一字段取代旧的 `category: Option<String>`）
///
/// 设计原则：
/// - `CapabilityCategory` 是**机制化的语义标签**，与具体语言无关
/// - `CapabilityMeta.category: Option<CapabilityCategory>` 是唯一分类字段
/// - 渲染层（`render_category`）按 `ctx.get("lang")` 选择本地化字符串
/// - 枚举新增 variant 时，老调用方的 `Some("xxx".to_string())` 形式已不可用，
///   必须迁移到 `Some(CapabilityCategory::Xxx)`（编译期强制）
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityCategory {
    /// 记忆管理：save / retrieve / list 等
    Memory,
    /// 智能推理：causal / logical / analogical 等
    Reasoning,
    /// 目标规划：decompose / generate / track
    Planning,
    /// 元认知：reflect / evaluate_decision
    Metacognition,
    /// 学习优化：extract / merge / decay
    Learning,
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
    /// 未分类：兜底
    #[default]
    Other,
}

impl CapabilityCategory {
    /// 默认本地化展示（中文）。后续可改为按 ctx.get("lang") 切换。
    /// 集中维护一处，避免散落硬编码。
    pub fn default_display(&self) -> &'static str {
        match self {
            Self::Memory => "记忆管理",
            Self::Reasoning => "智能推理",
            Self::Planning => "目标规划",
            Self::Metacognition => "元认知",
            Self::Learning => "学习优化",
            Self::Chat => "智能体协作",
            Self::Core => "核心能力",
            Self::Skill => "技能调用",
            Self::FileOperation => "文件操作",
            Self::Network => "网络搜索",
            Self::SystemOperation => "系统操作",
            Self::Mcp => "MCP 工具",
            Self::Other => "其他",
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
    /// 能力分类（v17：唯一分类字段，类型为枚举）
    ///
    /// - `Some(枚举)`：渲染层按 `ctx.get("lang")` 选本地化文案
    /// - `None`：兜底为 `Other`（展示"其他"）
    /// - **v17 变更**：旧 `Option<String>` 形态已废弃，编译期强制迁移
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub category: Option<CapabilityCategory>,
    /// 使用示例列表
    #[serde(skip_serializing_if = "Option::is_none")]
    pub examples: Option<Vec<String>>,
}

impl CapabilityMeta {
    /// 构造带 category 分类的元数据
    pub fn with_category(mut self, kind: CapabilityCategory) -> Self {
        self.category = Some(kind);
        self
    }

    /// 渲染本地化 category 文本
    ///
    /// 未来扩展点：当 `ctx.get("lang")` 可用时按语言切换
    /// （返回 `Cow<str>` 即可避免为中文/英文双重分配）。
    /// 当前阶段：直接返回 `default_display()`，兜底 `Other`。
    pub fn render_category(&self) -> &str {
        match self.category {
            Some(k) => k.default_display(),
            None => CapabilityCategory::Other.default_display(),
        }
    }

    /// 渲染 LLM 可见的 description（自动追加 examples）
    ///
    /// 协议层（openMODEL / anthropic / gemini）只需调用本方法，
    /// 即可让所有工具的 `examples` 字段真正送达 LLM。
    /// 无 examples 时直接返回原 description，零开销。
    pub fn description_for_llm(&self) -> String {
        match &self.examples {
            Some(exs) if !exs.is_empty() => {
                format!("{}\n\n示例：\n{}", self.description, exs.join("\n"))
            }
            _ => self.description.clone(),
        }
    }
}

#[async_trait]
pub trait Capability: Send + Sync + 'static {
    fn meta(&self) -> CapabilityMeta;

    fn name(&self) -> String {
        self.meta().name
    }

    /// 执行能力调用
    ///
    /// 参数通过 `ctx` 中的 payload 传递（使用 `InvokeRequestExt::payload()` 获取）
    async fn execute(&self, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload>;
}

#[async_trait]
pub trait CapabilityManager: Send + Sync + 'static {
    async fn register(&self, tool: Arc<dyn Capability>);

    async fn register_batch(&self, tools: Vec<Arc<dyn Capability>>) {
        for tool in tools {
            self.register(tool).await;
        }
    }

    async fn list_capability(&self) -> Vec<CapabilityMeta>;

    async fn invoke(
        &self,
        name: &str,
        ctx: Arc<dyn InvokeRequest>,
    ) -> InvokeResponse<PluginPayload>;

    async fn has_capability(&self, name: &str) -> bool;
}
