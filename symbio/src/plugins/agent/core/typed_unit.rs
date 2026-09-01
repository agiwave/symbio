//! CognitiveUnit（认知单元）
//!
//! **v9.4 设计重置**：CU 是 `serde_json::Map<String, Value>` 的薄包装。
//! 一切数据（id / name / 关系 / priority / 元数据 / 任何扩展）都存在 `data` 里。
//!
//! ## 设计目标
//!
//! - **单一数据源**：CU 的所有内容都是一个 `Map<String, Value>`，不再有"结构化字段 vs properties"
//!   的二元结构。`id` / `name` / `priority` / `is_a` / 任何自定义字段都是这个 Map 的平等成员。
//! - **函数式访问**：所有读写都通过方法（`unit.name()` / `unit.set_name(...)`），
//!   不暴露 `pub` 字段。这避免了"字段访问与 JSON 表示错位"的根本问题。
//! - **机制化**：没有"关系是特殊字段"、"扩展属性是 properties"等硬编码概念。
//!   关系（`is_a` / `causes` / 自定义关系）就是顶层字符串数组；任何其他键都是普通数据。
//! - **内部元数据**：仅以 `_ext_` 前缀标识（如 `_ext_version` / `_ext_embedding`），
//!   `to_llm_value` 过滤掉即可。无独立 `_meta` struct。

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::embedding_quant::{dequantize_from_value, q8_to_value};
use super::types::cu_fields;

/// 内部元数据键前缀（LLM 视图不暴露）
pub const META_PREFIX: &str = "_ext_";

// CognitiveUnit

/// 认知单元——`serde_json::Map<String, Value>` 的强类型薄包装
///
/// ## 数据契约
///
/// `data` 是单一数据源，**所有字段都在这里**：
/// - `id: String`（必需）
/// - `name: Option<String>`
/// - `description: Option<String>`
/// - `content: Option<String>`
/// - `is_a: Vec<String>`（以及任何自定义关系名）
/// - `priority: i64`（值小→先入提示词；>20→不进入；缺省 10）
/// - `confidence: f32`、`meta_belief: f32`
/// - `prop_value_is_a: Option<String>`（prop CU 的值类型约束）
/// - `_ext_version` / `_ext_created_at` / `_ext_updated_at` / `_ext_last_access` /
///   `_ext_access_count` / `_ext_memory_strength` / `_ext_embedding`（内部元数据）
/// - **任何**其他键——直接持久化（不存在"扩展属性应被 properties 隔离"的概念）
///
/// ## 序列化
///
/// `#[serde(flatten)]` 让 `data` 的所有键直接展平到顶层 JSON 对象上。
/// 序列化 CU 与序列化 `Map<String, Value>` 完全等价。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CognitiveUnit {
    #[serde(flatten)]
    data: Map<String, Value>,
}

// 构造

impl CognitiveUnit {
    /// 创建新 CU（必须指定 id）
    pub fn new(id: impl Into<String>) -> Self {
        let mut data = Map::new();
        data.insert(cu_fields::ID.to_string(), Value::String(id.into()));
        Self { data }
    }

    /// 生成随机 id 的新 CU
    pub fn generate_id() -> Self {
        let id = format!("cu_{}", &uuid::Uuid::new_v4().to_string()[..8]);
        Self::new(id)
    }

    /// 从 `serde_json::Value` 构造（必须是 Object 且含 `id`）
    pub fn from_value(v: Value) -> Result<Self, String> {
        let map = match v {
            Value::Object(m) => m,
            other => return Err(format!("Value must be an object, got {}", type_of(&other))),
        };
        Self::from_map(map)
    }

    /// 从 `serde_json::Map` 构造（含 `id` 字段是必需的）
    pub fn from_map(map: Map<String, Value>) -> Result<Self, String> {
        let id_missing = map
            .get(cu_fields::ID)
            .and_then(|v| v.as_str())
            .map(|s| s.is_empty())
            .unwrap_or(true);
        if id_missing {
            return Err("Missing or empty 'id' field".to_string());
        }
        Ok(Self { data: map })
    }

    /// 完整 `Value` 表示（包含 `_ext_*`），借用的视图
    pub fn to_value(&self) -> Value {
        Value::Object(self.data.clone())
    }

    /// 完整 `Value` 表示（消耗 self）
    pub fn into_value(self) -> Value {
        Value::Object(self.data)
    }

    /// 内部 Map 借用（不克隆）
    pub fn as_map(&self) -> &Map<String, Value> {
        &self.data
    }

    /// LLM 视图（过滤掉 `_ext_*`）
    pub fn to_llm_value(&self) -> Value {
        let map: Map<String, Value> = self
            .data
            .iter()
            .filter(|(k, _)| !k.starts_with(META_PREFIX))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        Value::Object(map)
    }
}

// id

impl CognitiveUnit {
    pub fn id(&self) -> &str {
        self.data
            .get(cu_fields::ID)
            .and_then(|v| v.as_str())
            .unwrap_or("")
    }

    pub fn set_id(&mut self, id: impl Into<String>) {
        self.data
            .insert(cu_fields::ID.to_string(), Value::String(id.into()));
    }
}

// name / description / content

impl CognitiveUnit {
    pub fn name(&self) -> Option<&str> {
        self.data.get(cu_fields::NAME).and_then(|v| v.as_str())
    }

    pub fn set_name(&mut self, name: impl Into<String>) {
        self.data
            .insert(cu_fields::NAME.to_string(), Value::String(name.into()));
    }

    pub fn clear_name(&mut self) {
        self.data.remove(cu_fields::NAME);
    }

    pub fn description(&self) -> Option<&str> {
        // 兼容：description 优先，缺失则回退到 content
        self.data
            .get(cu_fields::DESCRIPTION)
            .and_then(|v| v.as_str())
            .or_else(|| self.data.get(cu_fields::CONTENT).and_then(|v| v.as_str()))
    }

    pub fn set_description(&mut self, d: impl Into<String>) {
        self.data
            .insert(cu_fields::DESCRIPTION.to_string(), Value::String(d.into()));
    }

    pub fn clear_description(&mut self) {
        self.data.remove(cu_fields::DESCRIPTION);
    }

    pub fn content(&self) -> Option<&str> {
        self.data.get(cu_fields::CONTENT).and_then(|v| v.as_str())
    }

    pub fn set_content(&mut self, c: impl Into<String>) {
        self.data
            .insert(cu_fields::CONTENT.to_string(), Value::String(c.into()));
    }

    pub fn clear_content(&mut self) {
        self.data.remove(cu_fields::CONTENT);
    }
}

// priority / confidence / meta_belief

impl CognitiveUnit {
    pub fn confidence(&self) -> f32 {
        self.data
            .get(cu_fields::CONFIDENCE)
            .and_then(|v| v.as_f64())
            .map(|f| f as f32)
            .unwrap_or(0.5)
    }

    pub fn set_confidence(&mut self, v: f32) {
        self.data.insert(
            cu_fields::CONFIDENCE.to_string(),
            f64_to_value(v.clamp(0.0, 1.0) as f64),
        );
    }

    pub fn meta_belief(&self) -> f32 {
        self.data
            .get("meta_belief")
            .and_then(|v| v.as_f64())
            .map(|f| f as f32)
            .unwrap_or(0.5)
    }

    pub fn set_meta_belief(&mut self, v: f32) {
        self.data.insert(
            "meta_belief".to_string(),
            f64_to_value(v.clamp(0.0, 0.99) as f64),
        );
    }

    pub fn bump_meta_belief(&mut self, delta: f32) {
        let new_v = (self.meta_belief() + delta).min(0.99);
        self.set_meta_belief(new_v);
    }
}

// 关系（动态）
//
// 关系 = 顶层字符串数组（`is_a` / `causes` / `cures` / 任何自定义名）。
// 关系名无硬编码限制，可由 prop 注册表声明。

impl CognitiveUnit {
    /// 获取指定关系的所有目标（拷贝）
    pub fn relations(&self, name: &str) -> Vec<String> {
        self.data
            .get(name)
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 添加关系目标（去重）
    pub fn add_relation(&mut self, name: &str, target: &str) {
        let target_s = target.to_string();
        match self.data.get_mut(name) {
            Some(Value::Array(vec)) => {
                if !vec.iter().any(|x| x.as_str() == Some(&target_s)) {
                    vec.push(Value::String(target_s));
                }
            }
            Some(_) => {
                // 旧值不是数组，替换为单元素数组
                self.data.insert(
                    name.to_string(),
                    Value::Array(vec![Value::String(target_s)]),
                );
            }
            None => {
                self.data.insert(
                    name.to_string(),
                    Value::Array(vec![Value::String(target_s)]),
                );
            }
        }
    }

    /// 移除关系目标（清空后自动删除键）
    pub fn remove_relation(&mut self, name: &str, target: &str) {
        if let Some(Value::Array(vec)) = self.data.get_mut(name) {
            vec.retain(|x| x.as_str() != Some(target));
            if vec.is_empty() {
                self.data.remove(name);
            }
        }
    }

    /// 覆盖关系的所有目标
    pub fn set_relation(&mut self, name: &str, targets: Vec<String>) {
        if targets.is_empty() {
            self.data.remove(name);
        } else {
            self.data.insert(
                name.to_string(),
                Value::Array(targets.into_iter().map(Value::String).collect()),
            );
        }
    }

    pub fn has_relation(&self, name: &str, target: &str) -> bool {
        self.relations(name).iter().any(|t| t == target)
    }
}

// is_a 便捷

impl CognitiveUnit {
    pub fn is_a(&self) -> Vec<String> {
        self.relations(cu_fields::IS_A)
    }

    pub fn is_type(&self, t: &str) -> bool {
        self.is_a().iter().any(|x| x == t)
    }

    pub fn is_any_type(&self, types: &[&str]) -> bool {
        let is_a = self.is_a();
        is_a.iter().any(|t| types.iter().any(|&target| t == target))
    }

    pub fn add_type(&mut self, t: &str) {
        self.add_relation(cu_fields::IS_A, t);
    }

    pub fn remove_type(&mut self, t: &str) {
        self.remove_relation(cu_fields::IS_A, t);
    }

    pub fn set_types(&mut self, types: Vec<&str>) {
        let v: Vec<String> = types.into_iter().map(String::from).collect();
        self.set_relation(cu_fields::IS_A, v);
    }
}

// prop_value_is_a（prop CU 的"值类型约束"）

impl CognitiveUnit {
    pub fn prop_value_is_a(&self) -> Option<&str> {
        self.data.get("prop_value_is_a").and_then(|v| v.as_str())
    }

    /// 判定该 prop CU 是否定义了一个关系属性
    ///
    /// 规则（COGNITION.md §4.5）：
    /// 1. `is_a` 含 `relation`
    /// 2. `prop_value_is_a` ∈ {`cu`, `cu[]`}
    ///
    /// 同时满足 → 该属性名（即 `id`）是一个关系属性。
    pub fn is_relation_prop(&self) -> bool {
        if !self.is_type("relation") {
            return false;
        }
        matches!(self.prop_value_is_a(), Some("cu") | Some("cu[]"))
    }

    pub fn set_prop_value_is_a(&mut self, t: impl Into<String>) {
        self.data
            .insert("prop_value_is_a".to_string(), Value::String(t.into()));
    }

    pub fn clear_prop_value_is_a(&mut self) {
        self.data.remove("prop_value_is_a");
    }
}

// embedding（_ext_embedding）
//
// **v16 设计**：embedding 以 Q8 (per-vector affine) 量化格式存储。
// - 内存节省：512 维 f32 → 2048B → 520B（节省 75%）
// - 误差：≤ 1 LSB ≈ scale，对 bge-small-zh 等 512 维模型 ~0.5% 相对误差
// - 兼容：旧格式 `Value::Array<f32>` 自动按恒等映射读取（v19 修复后**不再**走 Q8 转换）
// - 写入：统一 Q8 v2 格式 `Value::Object { q8, scale, zero_point, _format_version: 2 }`
// - 与 EmbeddingStore 桶码 1-bit mean 量化（ANN 检索）正交：Q8 是存储层，桶码是索引层
//
// 详见 `embedding_quant.rs`。

impl CognitiveUnit {
    /// 读取 embedding（自动识别 Q8 格式与旧格式）
    ///
    /// 返回 `None` 当 `_ext_embedding` 字段缺失或格式无法识别。
    ///
    /// ## v19 修复
    /// 旧格式 `Value::Array<f32>` **不再**被当作 Q8 字节读，
    /// 改为**直接读为 f32 向量**（避免 `.round().clamp(0, 255)` 静默丢失负值/小数）。
    pub fn embedding(&self) -> Option<Vec<f32>> {
        let v = self.data.get("_ext_embedding")?;
        dequantize_from_value(v)
    }

    /// `embedding()` 的别名（语义化命名）
    pub fn get_embedding(&self) -> Option<Vec<f32>> {
        self.embedding()
    }

    /// 以 Q8 量化格式存储 embedding
    ///
    /// - 输入 NaN/Inf → 0（避免污染 min/max 统计）
    /// - 空向量 → 存 `{ q8: [], scale: 1.0, zero_point: 0.0, _format_version: 2 }`
    pub fn set_embedding(&mut self, e: Vec<f32>) {
        use super::embedding_quant::quantize_q8;
        // NaN/Inf → 0 替换（回归；与 quantize_q8 内部 sanitization 行为一致）
        let safe: Vec<f32> = e
            .into_iter()
            .map(|v| if v.is_finite() { v } else { 0.0 })
            .collect();
        let q = quantize_q8(&safe);
        self.data
            .insert("_ext_embedding".to_string(), q8_to_value(&q));
    }

    /// 直接以 Q8 字节写入（跳过重新量化，用于迁移/导入场景）
    ///
    /// 写入格式：`{ q8: [u8;N], scale: f32, zero_point: f32, _format_version: 2 }`
    ///
    /// ## v19 修复
    /// `scale` / `zero_point` 若为 NaN/Inf 会被 `q8_to_value` 内部替换为 0 并 warn，
    /// 不会让非有限值进入存储层（避免后续反量化产生 NaN 污染）。
    pub fn set_embedding_q8_raw(&mut self, data: Vec<u8>, scale: f32, zero_point: f32) {
        // v19：u8 类型天然 ≤ 255，无需运行时校验（编译期已保证）
        // 仅做 length > 0 的轻量校验（防止误传空数据）
        debug_assert!(!data.is_empty(), "Q8 data should not be empty");
        let obj = q8_to_value(&super::embedding_quant::Q8Embedding {
            data,
            scale,
            zero_point,
        });
        self.data.insert("_ext_embedding".to_string(), obj);
    }

    /// 检查当前 embedding 存储是否已是 Q8 格式
    pub fn is_embedding_q8(&self) -> bool {
        matches!(self.data.get("_ext_embedding"), Some(Value::Object(obj)) if obj.contains_key("q8"))
    }

    pub fn clear_embedding(&mut self) {
        self.data.remove("_ext_embedding");
    }
}

// 内部元数据（_ext_*）

impl CognitiveUnit {
    pub fn version(&self) -> u64 {
        self.data
            .get("_ext_version")
            .and_then(|v| v.as_u64())
            .unwrap_or(1)
    }

    pub fn bump_version(&mut self) {
        let v = self.version() + 1;
        self.data
            .insert("_ext_version".to_string(), Value::Number(v.into()));
        let now = now_secs();
        self.data
            .insert("_ext_updated_at".to_string(), Value::Number(now.into()));
    }

    pub fn record_access(&mut self) {
        let now = now_secs();
        self.data
            .insert("_ext_last_access".to_string(), Value::Number(now.into()));
        let count = self.access_count() + 1;
        self.data
            .insert("_ext_access_count".to_string(), Value::Number(count.into()));
    }

    pub fn last_access(&self) -> Option<u64> {
        self.data.get("_ext_last_access").and_then(|v| v.as_u64())
    }

    pub fn access_count(&self) -> u64 {
        self.data
            .get("_ext_access_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
    }

    pub fn memory_strength(&self) -> f64 {
        self.data
            .get("_ext_memory_strength")
            .and_then(|v| v.as_f64())
            .unwrap_or(24.0)
    }

    pub fn set_memory_strength(&mut self, s: f64) {
        self.data
            .insert("_ext_memory_strength".to_string(), f64_to_value(s));
    }

    pub fn created_at(&self) -> u64 {
        self.data
            .get("_ext_created_at")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
    }

    pub fn updated_at(&self) -> u64 {
        self.data
            .get("_ext_updated_at")
            .and_then(|v| v.as_u64())
            .unwrap_or(0)
    }

    /// v10：工作记忆 TTL 过期时间（Unix timestamp）
    /// 未设置则返回 `None`
    pub fn expires_at(&self) -> Option<u64> {
        self.data
            .get(cu_fields::EXPIRES_AT)
            .and_then(|v| v.as_u64())
    }

    /// v10：设置工作记忆 TTL 过期时间
    pub fn set_expires_at(&mut self, ts: u64) {
        self.data
            .insert(cu_fields::EXPIRES_AT.to_string(), Value::Number(ts.into()));
    }

    /// v10：判断工作记忆是否已过期
    ///
    /// **语义**：
    /// - 未设置 `expires_at` → 永不过期（`false`）
    /// - `now >= expires_at` → 已过期（`true`）
    /// - 用于 retrieve / list / count 自动过滤
    pub fn is_expired(&self, now: u64) -> bool {
        match self.expires_at() {
            Some(ts) => now >= ts,
            None => false,
        }
    }
}

// 通用数据访问（机制化核心）
//
// 任何键都可以通过 `get` / `set` / `remove` / `contains` 直接读写。
// 不存在"持久化扩展 vs 内存扩展"的概念——所有键平等。

impl CognitiveUnit {
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.data.get(key)
    }

    /// 取字符串值便捷方法
    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.data.get(key).and_then(|v| v.as_str())
    }

    /// 取数值便捷方法（f64，涵盖整数/浮点）
    pub fn get_number(&self, key: &str) -> Option<f64> {
        self.data.get(key).and_then(|v| v.as_f64())
    }

    pub fn set<K: Into<String>>(&mut self, key: K, value: Value) {
        self.data.insert(key.into(), value);
    }

    /// 写入字符串值便捷方法
    pub fn set_str<K: Into<String>>(&mut self, key: K, value: &str) {
        self.data
            .insert(key.into(), Value::String(value.to_string()));
    }

    /// `set_str` 的语义化别名（保持与历史 API 命名一致）
    pub fn set_property_str<K: Into<String>>(&mut self, key: K, value: &str) {
        self.set_str(key, value);
    }

    pub fn remove(&mut self, key: &str) -> Option<Value> {
        self.data.remove(key)
    }

    pub fn contains(&self, key: &str) -> bool {
        self.data.contains_key(key)
    }

    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.data.keys()
    }
}

// is_a 列表 / 设置便捷

impl CognitiveUnit {
    /// is_a 解析为 `Vec<CuRef>`（结构化引用，含可选别名）
    pub fn is_a_list(&self) -> Vec<crate::plugins::agent::core::types::CuRef> {
        use crate::plugins::agent::core::types::parse_cu_owned;
        self.is_a().iter().map(|s| parse_cu_owned(s)).collect()
    }

    /// 设置 is_a（覆盖式）
    pub fn set_is_a(&mut self, types: &[&str]) {
        self.set_types(types.to_vec());
    }
}

// 局部更新（apply_update）
//
// 来自 LLM 或 agent 的部分更新。
// 规则：
// - id 不可改（防止 ID 重写）
// - _ext_* 不可改（内部元数据由系统管理）
// - 其他键一律覆盖合并（包括结构化字段和未声明的扩展）
//
// 这是"机制化"原则的体现：CU 不区分"结构化字段"和"扩展属性"——
// 所有键都是 data 的一等公民，update 路径一律平等对待。

impl CognitiveUnit {
    pub fn apply_update(&mut self, value: &Value) -> Result<(), String> {
        let obj = value
            .as_object()
            .ok_or_else(|| "update value must be a JSON object".to_string())?;
        for (k, v) in obj {
            if k == cu_fields::ID {
                continue;
            }
            if k.starts_with(META_PREFIX) {
                continue;
            }
            self.data.insert(k.clone(), v.clone());
        }
        Ok(())
    }

    /// 基础结构校验（不依赖认知体系上下文）
    ///
    /// 仅校验字段类型和值范围。完整的体系化校验（属性名合法性、关系名注册、
    /// is_a 类型存在性、prop_value_is_a 约束）需要认知体系上下文，
    /// 由 ops 层的 `validate_cu_with_context` 负责。
    pub fn validate(&self) -> Result<(), String> {
        if self.id().is_empty() {
            return Err("id 不能为空".to_string());
        }
        if let Some(v) = self.data.get(cu_fields::IS_A) {
            match v {
                Value::Array(arr) => {
                    for (i, item) in arr.iter().enumerate() {
                        if !item.is_string() {
                            return Err(format!("is_a[{}] 必须是字符串，实际: {:?}", i, item));
                        }
                    }
                }
                _ => return Err(format!("is_a 必须是数组，实际: {:?}", v)),
            }
        }
        if let Some(v) = self.data.get("confidence") {
            if let Some(f) = v.as_f64() {
                if !(0.0..=1.0).contains(&f) {
                    return Err(format!("confidence 必须在 [0.0, 1.0] 范围内，实际: {}", f));
                }
            } else if !v.is_null() {
                return Err(format!("confidence 必须是数字，实际: {:?}", v));
            }
        }
        Ok(())
    }
}

// 文本嵌入

impl CognitiveUnit {
    pub fn text_for_embedding(&self) -> Option<String> {
        let mut parts = Vec::new();
        if let Some(name) = self.name() {
            if !name.is_empty() {
                parts.push(name.to_string());
            }
        }
        if let Some(desc) = self.description() {
            if !desc.is_empty() {
                parts.push(desc.to_string());
            }
        }
        if let Some(content) = self.content() {
            if !content.is_empty() {
                parts.push(content.to_string());
            }
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join(" "))
        }
    }
}

// 与 Value 互转

impl From<CognitiveUnit> for Value {
    fn from(u: CognitiveUnit) -> Self {
        Value::Object(u.data)
    }
}

impl TryFrom<Value> for CognitiveUnit {
    type Error = String;
    fn try_from(v: Value) -> Result<Self, Self::Error> {
        Self::from_value(v)
    }
}

// 相等性与哈希

impl PartialEq for CognitiveUnit {
    fn eq(&self, other: &Self) -> bool {
        // CU 的本质标识是 id（data 内容变化不影响相等性）
        self.id() == other.id()
    }
}

impl Eq for CognitiveUnit {}

impl std::hash::Hash for CognitiveUnit {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.id().hash(state);
    }
}

// 辅助函数

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// f64 → Value（处理 NaN/Inf，保留为 0）
fn f64_to_value(v: f64) -> Value {
    serde_json::Number::from_f64(v)
        .map(Value::Number)
        .unwrap_or(Value::Number(0.into()))
}

fn type_of(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

// 测试代码位于 `typed_unit_tests.rs`（同目录 sibling 文件，体积过大故外置）

#[cfg(test)]
#[path = "typed_unit_tests.rs"]
mod tests;
