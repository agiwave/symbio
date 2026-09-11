use crate::symbio_core::{InvokeRequest, InvokeResponse, ModelProvider, PluginPayload};
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

/// 工具上下文保留策略（会话机制化属性）
///
/// 工具在 `CapabilityMeta` 中声明"历史参数/结果在推理上下文中保留多少"，
/// 会话压缩层按声明通用处理，不对任何具体工具名做特殊化。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolContextRetention {
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

impl ToolContextRetention {
    /// 每个工具应保留的完整调用次数（`All` → `u32::MAX` 表示不限制）
    pub fn keep_count(self) -> u32 {
        match self {
            ToolContextRetention::All => u32::MAX,
            ToolContextRetention::LastN(n) => n.max(1),
            ToolContextRetention::LastOnly => 1,
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
    /// 工具上下文保留策略（`None` = 默认 `All`，完整参与滑动窗口）
    ///
    /// 机制说明：工具执行层把该策略 Stamp 到 ToolCall 消息 meta
    /// （`ctx_retention`），会话压缩层读取 meta 通用执行，会话侧
    /// 不感知任何具体工具名。
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub context_retention: Option<ToolContextRetention>,
}

impl CapabilityMeta {
    /// 构造带 category 分类的元数据
    pub fn with_category(mut self, kind: CapabilityCategory) -> Self {
        self.category = Some(kind);
        self
    }

    /// 构造带上下文保留策略的元数据
    pub fn with_context_retention(mut self, retention: ToolContextRetention) -> Self {
        self.context_retention = Some(retention);
        self
    }

    /// 读取生效的上下文保留策略（未声明视为 `All`）
    pub fn effective_context_retention(&self) -> ToolContextRetention {
        self.context_retention.unwrap_or_default()
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
pub trait CapabilityVisitor: Send + Sync + 'static {
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

    /// 注册模型服务（AI 对话能力）
    ///
    /// 纯 trait 契约后的语义：model 插件在 traverse 中按上下文（用户选中的
    /// 模型 id > 默认 provider > 首个启用）解析出**唯一生效**的
    /// `Arc<dyn ModelProvider>`（配置 + 协议钩子的绑定实现）并注册于此；
    /// 重复注册时后者覆盖（单槽）。会话发起时经同一次
    /// `traverse(TRAVERSE_AVAILABLE_TOOLS)` 广播，与工具、系统提示词一并收集。
    async fn register_model_provider(&self, provider: Arc<dyn ModelProvider>);

    /// 取当前生效的模型服务（trait object；协议适配细节内化于实现体）
    ///
    /// 会话引擎凭此实例即可直接发起 `execute_turn`（参数自含，无需回查
    /// model 插件内部注册表，也无需感知任何协议抽象）。
    async fn get_model_provider(&self) -> Option<Arc<dyn ModelProvider>>;

    /// 注册系统提示词（按名称保序；同名覆盖）
    async fn register_system_prompt(&self, name: &str, prompt: String);

    /// 列出已注册的系统提示词（(name, prompt)，保注册顺序）
    async fn list_system_prompts(&self) -> Vec<(String, String)>;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// serde 往返：声明字段序列化进 tools/list JSON；未声明字段不出现且旧 JSON 兼容
    #[test]
    fn context_retention_serde_roundtrip() {
        let meta = CapabilityMeta {
            name: "todo_write".into(),
            description: "d".into(),
            input_schema: serde_json::json!({"type": "object"}),
            context_retention: Some(ToolContextRetention::LastOnly),
            ..Default::default()
        };
        let v = serde_json::to_value(&meta).unwrap();
        assert_eq!(v["context_retention"], serde_json::json!("last_only"));
        let back: CapabilityMeta = serde_json::from_value(v).unwrap();
        assert_eq!(back.context_retention, Some(ToolContextRetention::LastOnly));

        // 旧 JSON（无该字段）反序列化兼容，且序列化时不输出空字段
        let old: CapabilityMeta = serde_json::from_value(serde_json::json!({
            "name": "x", "description": "d", "parameters": {}
        }))
        .unwrap();
        assert_eq!(old.context_retention, None);
        let ser = serde_json::to_value(&old).unwrap();
        assert!(ser.get("context_retention").is_none());
    }

    /// keep_count：All → u32::MAX，LastN(n) → n.max(1)，LastOnly → 1
    #[test]
    fn keep_count_semantics() {
        assert_eq!(ToolContextRetention::All.keep_count(), u32::MAX);
        assert_eq!(ToolContextRetention::LastN(0).keep_count(), 1);
        assert_eq!(ToolContextRetention::LastN(3).keep_count(), 3);
        assert_eq!(ToolContextRetention::LastOnly.keep_count(), 1);
    }
}
