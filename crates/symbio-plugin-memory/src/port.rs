//! `memory` 端口：**巩固债的执行体**（D-268 订正 / D-387 Q3）。
//!
//! ## 为什么是端口
//!
//! 巩固的触发条件看的是「一整轮结束后的活集」，没有事件格位；它的格位是债
//! （[`crate::debt`]）。执行体归端口的提供方——也就是记忆这个所有者自己：
//! 合并算法、活集折叠、两条事实形状都在本 crate，执行体只把它们串起来。
//!
//! ## 时序：编码先于巩固
//!
//! 执行体在债 `sent` **之后**跑；申报方只有在快照里已看见 `memory.encoded` 才欠这一行，
//! 所以执行时本轮编码已经在日志里，`seq` 拿得到。**不需要**「收轮后」的登记面，
//! 也不需要会话在 `settled` 之后再回来调一次。
//!
//! ## 读面是计划期快照
//!
//! 读面由宿主在计划那一圈固定（`D-376`）：执行体读到的就是「判它欠」的那份事实，
//! 不会因为并发写入而看到半截新状态。

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use symbio_plugin_api::port::{
    PortErrorClass, PortExecContext, PortExecutor, PortInput, PortOutcome, PortProvider,
};

/// 端口名：记忆自己的世界端口。端口名是数据，参与环事件 id 的端口位。
pub(crate) const MEMORY_PORT: &str = "memory";

/// 提供者 id（行名后缀）。
const PROVIDER_ID: &str = "memory";

pub(crate) struct MemoryPort;

impl PortProvider for MemoryPort {
    fn ports(&self) -> Vec<String> {
        vec![MEMORY_PORT.to_string()]
    }

    fn provider_id(&self) -> &str {
        PROVIDER_ID
    }

    fn executor(&self) -> Option<Arc<dyn PortExecutor>> {
        Some(Arc::new(MemoryPortExecutor))
    }
}

struct MemoryPortExecutor;

#[async_trait]
impl PortExecutor for MemoryPortExecutor {
    async fn execute(&self, ctx: PortExecContext, input: PortInput) -> PortOutcome {
        let Some(tag) = input.payload.get("tag").and_then(Value::as_str) else {
            // 行载荷由本插件自己的债表构造，缺 `tag` 说明申报侧坏了：不可重试。
            return PortOutcome::failed(PortErrorClass::NonRetryable, "巩固请求缺少 tag");
        };
        let Some(reader) = ctx.reader.as_ref() else {
            return PortOutcome::ok();
        };
        let events = reader.events();
        let now = events.iter().map(|e| e.ts).max().unwrap_or(0);
        let Some(drafts) = crate::consolidate::merge_next(&events, tag, now) else {
            return PortOutcome::ok();
        };
        // 草稿先缓冲在宿主写口，这一笔仍开着才一次性落格（`D-376`）。
        ctx.emit.emit(drafts).await;
        PortOutcome::ok()
    }
}

/// 端口行的登记（`port` 族的登记）。
pub(crate) fn memory_port_rows() -> Vec<symbio_plugin_api::CapabilityRow> {
    let provider: Arc<dyn PortProvider> = Arc::new(MemoryPort);
    symbio_plugin_api::port::port_rows(&provider)
}

#[cfg(test)]
#[path = "port.test.rs"]
mod tests;
