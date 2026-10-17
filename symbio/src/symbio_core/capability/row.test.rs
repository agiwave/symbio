//! `symbio_core/capability/row.rs` 的单元测试。
//!
//! 钉三件事：**行的名字从能力自述派生**（调用方没有机会填错）、**meta 现取**
//! （改名装饰器不会让表键与自述分叉）、**cap 拿到的是能力本身**（执行链路
//! 经审批闸那一层不被行Vocabulary截断）。

use super::*;
use crate::symbio_core::{CapabilityRiskLevel, ExecEnv, PluginError, PluginInvokeRequest};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

/// 最小能力桩：名字与风险档可指定，不发起真实执行。
struct StubTool {
    name: String,
}

#[async_trait]
impl Capability for StubTool {
    fn meta(&self) -> CapabilityMeta {
        CapabilityMeta {
            name: self.name.clone(),
            description: "桩".to_string(),
            input_schema: json!({"type": "object"}),
            risk: Some(CapabilityRiskLevel::Medium),
            ..Default::default()
        }
    }

    fn name(&self) -> String {
        self.name.clone()
    }

    async fn execute(
        &self,
        _args: Value,
        _env: &ExecEnv,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> Result<Value, PluginError> {
        Ok(json!({"ok": true}))
    }
}

/// 改名装饰器（与 `plugins/agent/host/scope.rs` 的 `PrefixedCapability` 同形）：
/// 只换 `meta().name`，执行原样透传。行词汇对它的全部要求是**不截断**。
struct Renamed {
    inner: Arc<dyn Capability>,
    displayed: String,
}

#[async_trait]
impl Capability for Renamed {
    fn meta(&self) -> CapabilityMeta {
        let mut m = self.inner.meta();
        m.name = self.displayed.clone();
        m
    }

    fn name(&self) -> String {
        self.displayed.clone()
    }

    async fn execute(
        &self,
        args: Value,
        env: &ExecEnv,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> Result<Value, PluginError> {
        self.inner.execute(args, env, ctx).await
    }
}

fn stub(name: &str) -> Arc<dyn Capability> {
    Arc::new(StubTool {
        name: name.to_string(),
    })
}

/// 行的名字 = 能力自述的 name（含挂载点前缀的全名），不是调用方另填的。
#[test]
fn a_tool_row_takes_its_name_from_the_capability_self_report() {
    let row = CapabilityRow::tool(stub("local/shell"));
    assert_eq!(row.name(), "local/shell");
    assert_eq!(row.kind(), CapabilityRowKind::Tool);
    assert_eq!(row.meta().name, "local/shell");
}

/// 改名装饰器：行键取自**装饰后**的自述，`cap()` 拿到的是装饰器本身——
/// 执行经它透传到内层，注册收口点的审批闸因此读到的也是装饰后的 meta（T-138
/// 透传纪律在词汇层的对应物）。
#[test]
fn a_renaming_decorator_is_not_cut_off_by_the_row_vocabulary() {
    let row = CapabilityRow::tool(Arc::new(Renamed {
        inner: stub("shell"),
        displayed: "agent_reviewer_shell".to_string(),
    }) as Arc<dyn Capability>);

    assert_eq!(row.name(), "agent_reviewer_shell");
    assert_eq!(row.meta().name, "agent_reviewer_shell");

    let cap = row.cap().expect("工具行的 cap 恒在");
    assert_eq!(cap.name(), "agent_reviewer_shell");
}

/// `cap()` 返回的能力与入表时是同一个实例（Arc 同一性）——
/// 「按名执行」拿到的是裹了审批闸的那一个，不是元数据的副本。
#[test]
fn cap_of_a_tool_row_is_the_very_capability_that_was_put_in() {
    let tool = stub("local/read");
    let row = CapabilityRow::tool(tool.clone());
    assert!(
        Arc::ptr_eq(row.cap().expect("工具行"), &tool),
        "行只持引用，不复制能力"
    );
}
