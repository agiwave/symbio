//! 能力收集期错误通道（跨插件共享设施）
//!
//! `Composite::traverse` 会吞掉子插件返回的 Err（`let _ = plugin.traverse(...)`），
//! 单插件因此**无法**靠返回值让整次能力收集失败。但有些失败是致命的、必须让会话
//! 立刻中止并明确报错——最典型的就是"会话绑定了一个不存在的智能体"。
//!
//! 解决：收集期错误写入 ctx 上的一个共享桶，由编排方（session 插件的能力收集管线
//! `plugins/session/chat_pipeline.rs`）在收集结束后统一取用判定。
//!
//! ## 归属规则
//!
//! 写侧是**任何参与 traverse 的插件**（当前为 agent 插件报子智能体装配硬错误），
//! 读侧是 session 编排方——插件之间互相不可见、不得直接引用（见 `plugins/mod.rs`
//! 的架构原则），故错误通道必须作为共享设施定义在 core。
//!
//! 反向规则：只被单一插件消费的设施定义在该插件内部，core 不承载单模块内部定义。
//! 收集管线本身（`collect_capabilities` / `attach_capabilities`）只有 session 插件
//! 调用（子智能体的嵌套会话经路由 `session/chat/send` 复用同一管线，
//! 不由 agent 直接驱动），因此定义在 session 插件内。

use crate::symbio_core::{InvokeRequest, InvokeRequestExt, SymbioKey};
use std::sync::Arc;
use tokio::sync::Mutex;

/// 能力收集期错误
#[derive(Debug, Clone)]
pub struct CapabilityError {
    /// 报错插件名（用于日志与错误信息定位）
    pub plugin: String,
    /// 人类可读的错误描述（会直接展示给用户）
    pub message: String,
}

/// 收集期错误桶在 `InvokeRequest` 中的类型安全键
pub struct CapabilityErrorsKey;

impl SymbioKey for CapabilityErrorsKey {
    type Value = Arc<Mutex<Vec<CapabilityError>>>;
    fn name(&self) -> &'static str {
        "capability_errors"
    }
    fn parse(&self, _s: &str) -> Option<Self::Value> {
        None
    }
    fn format(&self, _v: &Self::Value) -> String {
        "capability_errors".to_string()
    }
}

/// 收集期错误桶键常量
pub const CAPABILITY_ERRORS: CapabilityErrorsKey = CapabilityErrorsKey;

/// 初始化错误桶（由能力收集管线调用；重复调用无副作用）
pub fn init_error_bucket(ctx: &Arc<dyn InvokeRequest>) {
    if ctx.get(CAPABILITY_ERRORS).is_none() {
        ctx.set(CAPABILITY_ERRORS, Arc::new(Mutex::new(Vec::new())));
    }
}

/// 插件在 `traverse` 中报告一个**致命**收集错误。
///
/// 只用于"会话无法继续"的硬错误（如选定的智能体不存在）。
/// 可降级的软故障（某个 MCP server 连不上）应当只记日志，不调用本函数。
pub async fn report_error(ctx: &Arc<dyn InvokeRequest>, plugin: &str, message: impl Into<String>) {
    init_error_bucket(ctx);
    if let Some(bucket) = ctx.get(CAPABILITY_ERRORS) {
        bucket.lock().await.push(CapabilityError {
            plugin: plugin.to_string(),
            message: message.into(),
        });
    }
}

/// 取出并清空所有收集期错误
pub async fn take_errors(ctx: &Arc<dyn InvokeRequest>) -> Vec<CapabilityError> {
    match ctx.get(CAPABILITY_ERRORS) {
        Some(bucket) => std::mem::take(&mut *bucket.lock().await),
        None => Vec::new(),
    }
}
