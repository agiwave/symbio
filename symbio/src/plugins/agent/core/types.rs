use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::typed_unit::CognitiveUnit;

/// 通用操作结果类型
///
/// 用于能力层（capabilities）各工具返回统一的结果格式。
/// 替代之前 MemoryResult、ReasonResult、LearnResult、PlanResult 等重复类型。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationResult {
    pub success: bool,
    pub data: Option<Value>,
    pub error: Option<String>,
}

impl OperationResult {
    pub fn success(data: Value) -> Self {
        Self {
            success: true,
            data: Some(data),
            error: None,
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            success: false,
            data: None,
            error: Some(message.into()),
        }
    }
}

/// 获取当前 Unix 时间戳（秒）
///
/// 统一的时间戳获取函数，避免在多处重复实现。
pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// CognitiveUnit 内部扩展字段常量
///
/// 所有以 `_ext_` 开头的字段是内部使用字段，永远不向 LLM 暴露
pub mod cu_fields {
    // ── 核心字段（I-014: 从字符串字面量统一为常量） ──
    /// 唯一标识符
    pub const ID: &str = "id";
    /// 显示名称
    pub const NAME: &str = "name";
    /// 详细描述
    pub const DESCRIPTION: &str = "description";
    /// 正文内容
    pub const CONTENT: &str = "content";
    /// 置信度（0.0 ~ 1.0）
    pub const CONFIDENCE: &str = "confidence";
    /// 类型继承关系（is_a = ["fact", "rule"]）
    pub const IS_A: &str = "is_a";
    /// 用户标签
    #[allow(dead_code)]
    pub const TAGS: &str = "tags";
    /// 元认知优先级（值越小越重要/越先入提示词，缺省 10）
    ///
    /// **核心作用**：
    /// - 决定 CU 是否进入系统提示词：`priority <= ENTER_PROMPT_THRESHOLD`（默认 20）才能进
    /// - 决定 CU 在系统提示词内的排序：值小的排前（**只在同 is_a/kind 内比较**）
    /// - LLM 写新 CU 时**不需要**考虑特殊字段，只设 `priority` 即可：
    ///   - 想强制排到同 kind 内最前：`priority = 0`
    ///   - 默认：让系统自动分配 10（候选池内居中）
    ///   - 不进系统提示词：`priority = 30`（超出阈值不进入）
    pub const PRIORITY: &str = "priority";

    // ── 内部元数据字段（_ext_ 前缀，不暴露给 LLM） ──
    /// 版本号字段
    #[allow(dead_code)]
    pub const VERSION: &str = "_ext_version";
    /// 创建时间（Unix timestamp）
    #[allow(dead_code)]
    pub const CREATED_AT: &str = "_ext_created_at";
    /// 更新时间（Unix timestamp）
    #[allow(dead_code)]
    pub const UPDATED_AT: &str = "_ext_updated_at";
    /// 过期时间（Unix timestamp，v10 工作记忆 TTL）
    ///
    /// 设置后，retrieve / list / count 应自动过滤掉 `now >= expires_at` 的单元。
    /// 与 `is_a = working_memory` 配合使用。
    pub const EXPIRES_AT: &str = "_ext_expires_at";
}

pub fn generate_short_id() -> String {
    uuid::Uuid::new_v4().as_simple().to_string()[..8].to_string()
}

// 迁移（v8 决策：直接统一为强类型）
//
// **v9.4 重构**：CU 是 `Map<String, Value>` 的薄包装。
// 所有访问通过方法完成；不再有"结构化字段 vs properties"的二元结构。

#[derive(Debug, Clone)]
pub struct CuRef {
    pub id: String,
    pub name: Option<String>,
}

pub fn parse_cu_ref(ref_str: &str) -> CuRef {
    if let Some(colon_pos) = ref_str.find("::") {
        if colon_pos > 0 && colon_pos + 2 < ref_str.len() {
            let name = &ref_str[..colon_pos];
            let id = &ref_str[colon_pos + 2..];
            if !id.contains('/') && !id.contains('\\') {
                return CuRef {
                    id: id.to_string(),
                    name: Some(name.to_string()),
                };
            }
        }
    }
    CuRef {
        id: ref_str.to_string(),
        name: None,
    }
}

/// parse_cu_ref 的 owned 入参版本：用于处理 `String` 切片（如 CU data 字段）
pub fn parse_cu_owned(ref_str: &str) -> CuRef {
    parse_cu_ref(ref_str)
}

#[cfg(test)]
pub fn new_cognitive_unit() -> CognitiveUnit {
    CognitiveUnit::new(generate_short_id())
}

/// 规范化待存储的 CU：id 为空时自动生成 short id，否则原样返回（owned）。
///
/// 五个 store 后端（memory / file / dir / sqlite / embedding_store）在 `upsert`
/// 入口都重复了 "if id.is_empty() { clone; set_id(generate_short_id()); insert }"
/// 的样板；本函数抽出该共性，让各后端只关心 insert/update 分支。
///
/// 返回的 CU 一定带有非空 id，可安全用作 `insert` / `update` / `get` 的 key。
pub fn unit_with_id(unit: &CognitiveUnit) -> CognitiveUnit {
    if unit.id().is_empty() {
        let mut u = unit.clone();
        u.set_id(generate_short_id());
        u
    } else {
        unit.clone()
    }
}

/// 从 JSON 构造 CognitiveUnit
///
/// 原 `match ... { Err(_) => ... }` 静默吞错，调试时无法定位数据格式异常。
/// v28 改为在 fallback 路径加 `plugin_warn!`，把失败原因写到日志。
pub fn cu_from_json(value: Value) -> CognitiveUnit {
    match CognitiveUnit::try_from(value.clone()) {
        Ok(u) => u,
        Err(e) => {
            // 失败时记录错误原因 + 触发原因（关键字段缺失 / 字段类型错）
            crate::plugin_warn!(
                "agent",
                "[cu_from_json] CognitiveUnit::try_from 失败，使用 fallback：err={}，\
                value_keys={:?}（数据持久化层兼容性兜底，提示 schema 不匹配）",
                e,
                value
                    .as_object()
                    .map(|o| o.keys().cloned().collect::<Vec<_>>())
            );
            // 兜底：id 缺失时生成一个，把原数据塞回去
            let mut fallback = CognitiveUnit::new(generate_short_id());
            if let Some(obj) = value.as_object() {
                for (k, v) in obj {
                    fallback.set(k, v.clone());
                }
            }
            fallback
        },
    }
}

// 注：v9.5 决策废除 `CognitiveUnitExt` 后，所有便捷方法（get_str / get_number /
// set_str / is_a_list / set_is_a / get_embedding / 带 content 回退的 description）
// 已直接合并到 `CognitiveUnit` 的 inherent impl 中。Trait 不再需要——
// 调用方直接 `cu.method()` 即可。

// 注：v8 决策"直接统一"后，`CognitiveUnit = CognitiveUnit`，适配器不再是必须的。
// 上层若需要 `Value` ↔ `CognitiveUnit` 转换，可直接用 `CognitiveUnit::try_from(value)` /
// `value.into()`，或 `cu.into_value()`。原 `cu_from_value` / `cu_try_from_value` /
// `cu_to_value` / `cu_to_value_ref` 等 wrapper 已删除。

/// 按字符（不是字节）截断字符串，超出部分用 "..." 替代
///
/// 消除 `engine/conversation.rs::truncate` 与 `handlers/context_builder.rs::truncate_str`
/// 两份等价但实现略有差异的副本，统一为按 Unicode 标量值（char）截断
///
/// 行为：
/// - `text.chars().count() <= max_chars` → 原样返回
/// - 否则取前 `max_chars` 个字符 + "..."
pub fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let truncated: String = text.chars().take(max_chars).collect();
    format!("{}...", truncated)
}

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
