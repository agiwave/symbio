//! `symbio_core/capability/row.rs` 的单元测试。
//!
//! 钉三件事：**行的名字从能力自述派生**（调用方没有机会填错）、**meta 现取**
//! （改名装饰器不会让表键与自述分叉）、**cap 拿到的是能力本身**（执行链路
//! 经审批闸那一层不被行Vocabulary截断）。

use super::*;
use crate::capability::{CapabilityMeta, CapabilityRiskLevel};
use crate::chat::ChatNode;
use crate::exec::ExecEnv;
use crate::llm::TurnOutput;
use crate::plugin::{PluginError, PluginInvokeRequest};
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

/// 最小模型服务桩：只提供 `provider_id`（行名就从它派生），执行永不到达。
struct StubProvider(&'static str);

#[async_trait]
impl ModelProvider for StubProvider {
    fn provider_id(&self) -> &str {
        self.0
    }

    fn api_protocol(&self) -> &str {
        "openai_chat"
    }

    fn rate_limit_ms(&self) -> u64 {
        0
    }

    fn max_context_tokens(&self) -> u32 {
        262_144
    }

    async fn effective_context_tokens(&self) -> u32 {
        262_144
    }

    async fn round_trip(
        &self,
        _system_prompt: &str,
        _messages: &[ChatNode],
        _tools: &[CapabilityMeta],
        _root_id: &str,
        _env: &ExecEnv,
    ) -> Result<TurnOutput, PluginError> {
        Err(PluginError::InternalError("stub".to_string()))
    }
}

/// 行的名字 = 能力自述的 name（含挂载点前缀的全名），不是调用方另填的。
#[test]
fn a_tool_row_takes_its_name_from_the_capability_self_report() {
    let row = CapabilityRow::tool(stub("local/shell"));
    assert_eq!(row.name(), "local/shell");
    assert_eq!(row.family(), family::TOOL);
    assert_eq!(row.meta().name, "local/shell");
}

/// 改名装饰器：行键取自**装饰后**的自述，`cap()` 拿到的是装饰器本身——
/// 执行经它透传到内层，注册收口点的审批闸因此读到的也是装饰后的 meta（
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

/// 新族在**声明方**定义：一个 [`CapabilityFamily`] 常量 + 自己的载荷类型。
///
/// 行词汇只按族过滤、按具体类型下转型——内核没有它的枚举臂，这就是「加一族不改
/// 内核」的最小证据。
#[test]
fn a_family_declared_outside_core_round_trips_its_payload() {
    struct NotePayload {
        text: String,
    }
    const NOTE: CapabilityFamily = CapabilityFamily::multi("test_note");

    let row = CapabilityRow::new(
        NOTE,
        "hello",
        NotePayload {
            text: "正文".to_string(),
        },
        false,
    );

    assert_eq!(row.family(), NOTE);
    assert_eq!(row.family().name(), "test_note");
    assert!(!row.family().is_single());
    assert_eq!(
        row.payload::<NotePayload>().map(|p| p.text.as_str()),
        Some("正文")
    );
    // 读错类型 ⇒ 诚实的 None（不 panic，也不误认成别的族）
    assert!(row.payload::<String>().is_none());
    assert!(row.cap().is_none(), "开放载荷不会冒充工具行");
}

/// 模型行的名字 = `provider_id()`（表键）；生效位是行的一部分。
///
/// `cap()` 为空而 `provider()` 在，两个写入方（工具 / 模型）在同一个名字空间里
/// 也不会互相冒充。
#[test]
fn a_model_row_takes_its_name_from_the_provider_id() {
    let row = CapabilityRow::model(
        Arc::new(StubProvider("openai")) as Arc<dyn ModelProvider>,
        true,
    );
    assert_eq!(row.name(), "openai");
    assert_eq!(row.family(), family::MODEL);
    assert_eq!(row.provider().expect("模型行").provider_id(), "openai");
    assert!(row.cap().is_none(), "模型行不是可调用能力");
    assert!(row.active(), "带位的行才占生效槽");
}

/// `demote()` 给的是**退位副本**：位清零、名字与 provider 不变（同一个实例），
/// 原行不动——表里的行是 `Arc`，单槽移交靠换副本入表，不靠就地改位。
#[test]
fn demote_returns_a_copy_with_the_slot_cleared() {
    let provider = Arc::new(StubProvider("openai")) as Arc<dyn ModelProvider>;
    let row = CapabilityRow::model(provider.clone(), true);

    let demoted = row.demote();
    assert!(!demoted.active());
    assert_eq!(demoted.name(), "openai");
    assert!(
        Arc::ptr_eq(demoted.provider().expect("模型行"), &provider),
        "退位只清位，不换 provider 实例"
    );
    assert!(row.active(), "原行不受影响");
    // 无槽的族：退位副本就是它自己（工具行没有位可清）
    let tool_row = CapabilityRow::tool(stub("local/read"));
    assert!(!tool_row.active());
    assert_eq!(tool_row.demote().name(), "local/read");
}
