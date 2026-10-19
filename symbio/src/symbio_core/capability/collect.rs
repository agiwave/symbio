//! 收集管线的**读侧助手**：按 ctx 取工具元数据清单。
//!
//! 原先住 session 的 `capabilities.rs`，四个读点共用（LLM 工具面 / 请求级开销
//! 估算 / 闸门声明位 / 线上名解析）。[ADR-086](../../../../docs/010-adr/086.md "ADR-086")
//! §7 上移 core：驱动器切成独立插件后，工具面读取在 session 与 loop 两侧都要用。
//!
//! 顺序 = 注册顺序（ADR-057），调用方不得重排；取不到收集器落空表。

use std::sync::Arc;

use crate::symbio_core::{
    CapabilityMeta, CapabilityRowKind, PluginInvokeRequest, PluginInvokeRequestExt,
    CAPABILITY_VISITOR,
};

pub async fn tool_metas(ctx: &Arc<dyn PluginInvokeRequest>) -> Vec<CapabilityMeta> {
    match ctx.get(CAPABILITY_VISITOR) {
        Some(visitor) => visitor
            .list(CapabilityRowKind::Tool)
            .await
            .into_iter()
            .map(|row| row.meta())
            .collect(),
        None => Vec::new(),
    }
}
