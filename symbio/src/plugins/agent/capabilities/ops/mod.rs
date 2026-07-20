//! 操作级拆分模块
//!
//! 每个认知操作是一个独立的 `.rs` 文件，实现统一的 [`CognitionOp`] trait，
//! 并通过 [`submit_cognition_op!`] 宏自注册到全局注册表。
//!
//! ## 自注册机制
//!
//! 每个 op 文件末尾调用 `submit_cognition_op!` 宏即可完成注册，无需修改任何中间模块：
//!
//! ```ignore
//! // ops/memory/save.rs
//! impl_cognition_op!(SaveOp, "memory.save", "保存单条记忆。范例：...", execute_save);
//! submit_cognition_op!(SaveOp);
//! ```
//!
//! ## 目录结构
//!
//! ```text
//! ops/
//! ├── mod.rs              # trait 定义 + 注册表 + 宏
//! ├── memory/             # memory 域操作（CRUD + 语义搜索 + 图关系搜索）
//! └── learn/              # learn 域操作（update, stats）
//! ```

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;

use crate::plugins::agent::core::{AgentStore, OperationResult};
use crate::symbio_core::CapabilityMeta;

// 子模块声明
pub(crate) mod memory;

// ═══════════════════════════════════════════════════════════════════════════
// CognitionOp trait
// ═══════════════════════════════════════════════════════════════════════════

/// 认知操作统一接口
///
/// 复用系统 [`CapabilityMeta`] 作为元数据容器，避免与 `Capability` trait 重复定义。
/// 每个操作实现此 trait，提供：
/// - `meta()`: 元数据（name / description / input_schema / examples 等）
/// - `execute()`: 执行逻辑（参数缺失时返回使用提示）
#[async_trait]
pub(crate) trait CognitionOp: Send + Sync {
    /// 操作元数据，复用系统 [`CapabilityMeta`]
    fn meta(&self) -> CapabilityMeta;

    /// 执行操作
    ///
    /// - `engine`: 认知引擎引用
    /// - `params`: 统一请求参数（来自 CognitionRequest 的 serde_json::Value）
    ///
    /// 参数缺失时，应返回 `OperationResult::error` 并附带使用提示（含范例）。
    async fn execute(&self, engine: Arc<dyn AgentStore>, params: &Value) -> OperationResult;
}

// ═══════════════════════════════════════════════════════════════════════════
// 自注册机制（基于 inventory）
// ═══════════════════════════════════════════════════════════════════════════

/// 认知操作提交器（用于 inventory 分布式收集）
pub(crate) struct CognitionOpSubmit {
    pub(crate) op: fn() -> Arc<dyn CognitionOp>,
}

inventory::collect!(CognitionOpSubmit);

/// 自注册宏：每个 op 文件末尾调用即可完成注册
///
/// # 用法
///
/// ```ignore
/// // 1. 先用 impl_cognition_op! 定义操作
/// impl_cognition_op!(SaveOp, "memory.save", "保存单条记忆。范例：...", execute_save);
///
/// // 2. 末尾调用 submit_cognition_op! 自注册
/// submit_cognition_op!(SaveOp);
/// ```
#[macro_export]
macro_rules! submit_cognition_op {
    ($struct_name:ident) => {
        $crate::symbio_core::inventory::submit! {
            $crate::plugins::agent::capabilities::ops::CognitionOpSubmit {
                op: (|| {
                    std::sync::Arc::new($struct_name) as std::sync::Arc<dyn $crate::plugins::agent::capabilities::ops::CognitionOp>
                }) as fn() -> std::sync::Arc<dyn $crate::plugins::agent::capabilities::ops::CognitionOp>,
            }
        }
    };
}

// ═══════════════════════════════════════════════════════════════════════════
// 操作注册表
// ═══════════════════════════════════════════════════════════════════════════

/// 全局操作注册表
///
/// 通过 `inventory::iter` 自动收集所有通过 `submit_cognition_op!` 注册的操作。
/// 纯查找职责：schema 生成、参数校验等上层逻辑由 `AgentCognitionTool` 负责。
pub(crate) struct OpRegistry {
    ops: HashMap<String, Arc<dyn CognitionOp>>,
}

impl OpRegistry {
    /// 按操作名查找
    pub(crate) fn get(&self, op_name: &str) -> Option<&Arc<dyn CognitionOp>> {
        self.ops.get(op_name)
    }

    /// 列出所有已注册的操作名
    pub(crate) fn registered_ops(&self) -> Vec<&str> {
        self.ops.keys().map(|s| s.as_str()).collect()
    }

    /// 遍历所有已注册的操作（供上层构建 schema 等）
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&String, &Arc<dyn CognitionOp>)> {
        self.ops.iter()
    }
}

use std::sync::OnceLock;

/// 全局操作注册表单例
static OP_REGISTRY: OnceLock<OpRegistry> = OnceLock::new();

/// 获取全局操作注册表（通过 inventory 自动收集所有自注册的操作）
pub(crate) fn get_registry() -> &'static OpRegistry {
    OP_REGISTRY.get_or_init(|| {
        let mut ops: HashMap<String, Arc<dyn CognitionOp>> = HashMap::new();

        // 通过 inventory 自动收集所有 submit_cognition_op! 注册的操作
        for submit in inventory::iter::<CognitionOpSubmit> {
            let op = (submit.op)();
            ops.insert(op.meta().name.clone(), op);
        }

        OpRegistry { ops }
    })
}

/// 辅助宏：快速实现 CognitionOp trait
///
/// 用法（4 参数，无 schema）：
/// ```ignore
/// impl_cognition_op!(SaveOp, "memory.save", "保存单条记忆。范例：...", save_execute);
/// submit_cognition_op!(SaveOp);
/// ```
///
/// 用法（5 参数，含 schema 函数）：
/// ```ignore
/// fn save_schema() -> serde_json::Value { serde_json::json!({...}) }
/// impl_cognition_op!(SaveOp, "memory.save", "保存单条记忆。范例：...", save_execute, save_schema);
/// submit_cognition_op!(SaveOp);
/// ```
#[macro_export]
macro_rules! impl_cognition_op {
    // 4 参数版本：使用默认 schema（空对象）
    ($struct_name:ident, $op_name:expr, $description:expr, $execute_fn:ident) => {
        pub struct $struct_name;

        #[async_trait::async_trait]
        impl $crate::plugins::agent::capabilities::ops::CognitionOp for $struct_name {
            fn meta(&self) -> $crate::symbio_core::CapabilityMeta {
                $crate::symbio_core::CapabilityMeta {
                    name: $op_name.to_string(),
                    description: $description.to_string(),
                    input_schema: serde_json::json!({}),
                    ..Default::default()
                }
            }

            async fn execute(
                &self,
                engine: std::sync::Arc<dyn $crate::plugins::agent::core::AgentStore>,
                params: &serde_json::Value,
            ) -> $crate::plugins::agent::core::OperationResult {
                $execute_fn(engine, params).await
            }
        }
    };

    // 5 参数版本：使用自定义 schema 函数
    ($struct_name:ident, $op_name:expr, $description:expr, $execute_fn:ident, $schema_fn:ident) => {
        pub struct $struct_name;

        #[async_trait::async_trait]
        impl $crate::plugins::agent::capabilities::ops::CognitionOp for $struct_name {
            fn meta(&self) -> $crate::symbio_core::CapabilityMeta {
                $crate::symbio_core::CapabilityMeta {
                    name: $op_name.to_string(),
                    description: $description.to_string(),
                    input_schema: $schema_fn(),
                    ..Default::default()
                }
            }

            async fn execute(
                &self,
                engine: std::sync::Arc<dyn $crate::plugins::agent::core::AgentStore>,
                params: &serde_json::Value,
            ) -> $crate::plugins::agent::core::OperationResult {
                $execute_fn(engine, params).await
            }
        }
    };
}

// ═══════════════════════════════════════════════════════════════════════════
// 测试
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests;
