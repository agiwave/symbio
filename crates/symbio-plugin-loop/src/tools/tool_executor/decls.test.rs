//! 工具自声明的行为位——三道闸门的管辖对象从注册表读，不按名字判。
//!
//! 钉四件事：
//!
//! - **各族只判自己的工具**（`agent_run` 两位命中、`todo_write` 一位命中、别的
//!   工具一位都不命中）——机制化前后行为逐格一致的直接判据；
//! - **匹配口径保持机制化之前**（线上全名等值 / 取末段比短名、trim、空串不命中）；
//! - **声明随工具进子作用域**（改名投影只换名字、原样透传行为位）——闸门跟随
//!   工具，这是「换个会话绕不开淘汰」要的读法；
//! - **没有 visitor ⇒ 什么都不命中**（缺声明与漏声明同形，e2e 兜行为）。

use super::ToolDecls;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use symbio_core::{
    Capability, CapabilityMeta, CapabilityRow, ExecEnv, PluginError, PluginInvokeRequest,
    PluginInvokeRequestExt, PluginSimpleRequest, Registry,
};
use symbio_providers::DefaultToolVisitor;

/// 只带元数据的假工具：注册表怎么读，测试就怎么喂。
struct FakeTool {
    meta: CapabilityMeta,
}

#[async_trait]
impl Capability for FakeTool {
    fn meta(&self) -> CapabilityMeta {
        self.meta.clone()
    }

    async fn execute(
        &self,
        _args: Value,
        _env: &ExecEnv,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> Result<Value, PluginError> {
        Ok(Value::Null)
    }
}

/// 按行为名构造一份元数据（其余字段走 `Default`）。
fn meta(name: &str, assign_work: bool, promise: bool, task_table: bool) -> CapabilityMeta {
    use symbio_plugin_api::behaviors::{ASSIGN_WORK, DELEGATION_PROMISE, TASK_TABLE};
    CapabilityMeta {
        name: name.to_string(),
        behaviors: match (assign_work, promise, task_table) {
            (false, false, false) => &[],
            (true, false, false) => &[ASSIGN_WORK],
            (false, true, false) => &[DELEGATION_PROMISE],
            (false, false, true) => &[TASK_TABLE],
            (true, true, false) => &[ASSIGN_WORK, DELEGATION_PROMISE],
            (true, false, true) => &[ASSIGN_WORK, TASK_TABLE],
            (false, true, true) => &[DELEGATION_PROMISE, TASK_TABLE],
            (true, true, true) => &[ASSIGN_WORK, DELEGATION_PROMISE, TASK_TABLE],
        },
        ..Default::default()
    }
}

/// 带 `CAPABILITY_VISITOR` 的上下文：按序注册给定的元数据。
async fn ctx_with(metas: Vec<CapabilityMeta>) -> Arc<dyn PluginInvokeRequest> {
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    let visitor = Arc::new(DefaultToolVisitor::new());
    for m in metas {
        visitor
            .put(CapabilityRow::tool(
                Arc::new(FakeTool { meta: m }) as Arc<dyn Capability>
            ))
            .await;
    }
    ctx.set(
        symbio_core::CAPABILITY_VISITOR,
        visitor as Arc<dyn Registry>,
    );
    ctx
}

/// 生产三件套的注册面：`agent_run`（派活 + 立约）、`todo_write`（任务表）、一个
/// 什么都不声明的普通工具（生产里绝大多数工具的形态）。
fn factory_metas() -> Vec<CapabilityMeta> {
    vec![
        meta("agent_run", true, true, false),
        meta("todo_write", false, false, true),
        meta("shell", false, false, false),
    ]
}

// ==================== 各族只判自己的工具 ====================

/// 三族判据互不串门：`agent_run` 命中前两位、`todo_write` 命中第三位、
/// 未声明的 `shell` 一位都不命中。
#[tokio::test]
async fn each_family_judges_only_its_own_tool() {
    let ctx = ctx_with(factory_metas()).await;
    let decls = ToolDecls::of_metas(&symbio_core::tool_metas(&ctx).await);

    assert!(decls.provides_assign_work("agent_run"));
    assert!(decls.records_delegation_promise("agent_run"));
    assert!(!decls.owns_task_table("agent_run"), "派活 ≠ 任务表");

    assert!(decls.owns_task_table("local/todo_write"));
    assert!(!decls.provides_assign_work("local/todo_write"));
    assert!(!decls.records_delegation_promise("local/todo_write"));

    assert!(
        !decls.provides_assign_work("local/shell")
            && !decls.records_delegation_promise("local/shell")
            && !decls.owns_task_table("local/shell"),
        "未声明任何位的工具不在任何闸门管辖内"
    );
}

// ==================== 匹配口径保持机制化之前 ====================

/// 匹配口径逐字保持：线上全名**等值**（派活 / 立约，含 trim）与**取末段比短名**
/// （任务表，`local/todo_write` ⇒ `todo_write`）。
#[tokio::test]
async fn matching_keeps_the_pre_mechanization_shapes() {
    let ctx = ctx_with(factory_metas()).await;
    let decls = ToolDecls::of_metas(&symbio_core::tool_metas(&ctx).await);

    // 等值口径：空格不算名字的一部分，前缀名不是等值命中。
    assert!(decls.provides_assign_work(" agent_run "));
    assert!(!decls.provides_assign_work("local/agent_run"));

    // 取末段口径：短名本身与挂载点前缀两种形态都认；近似名不认。
    assert!(decls.owns_task_table("todo_write"));
    assert!(decls.owns_task_table("local/todo_write"));
    assert!(decls.owns_task_table(" agent_scope/todo_write "));
    assert!(!decls.owns_task_table("todo_writer"));
    assert!(!decls.owns_task_table("local/todo_writer"));
    assert!(!decls.owns_task_table(""));
    assert!(!decls.provides_assign_work(""));
}

// ==================== 声明随工具进子作用域 ====================

/// 改名投影（子智能体的 `agent_<id>_<工具>`）只换名字、原样透传行为名，
/// 所以闸门在子作用域**同样命中**——此前按字面量等值判，前缀名一个都不中。
///
/// 这不是扩面的口子，是「闸门跟随工具」的直读：淘汰判据本来就部署级
/// （扫全部会话的 WAL），没有理由在子会话里失明。
#[tokio::test]
async fn declarations_travel_into_scoped_projection_names() {
    let ctx = ctx_with(vec![
        meta("agent_probe-agent_agent_run", true, true, false),
        meta("agent_probe-agent_todo_write", false, false, true),
    ])
    .await;
    let decls = ToolDecls::of_metas(&symbio_core::tool_metas(&ctx).await);

    assert!(decls.provides_assign_work("agent_probe-agent_agent_run"));
    assert!(decls.records_delegation_promise("agent_probe-agent_agent_run"));
    assert!(decls.owns_task_table("agent_probe-agent_todo_write"));
}

// ==================== 没有 visitor ====================

/// 没有 visitor ⇒ 三个集合皆空 ⇒ 三道闸都不命中。
///
/// 与「该声明的工具漏了声明」同一形态——都由 e2e 兜行为（T74 的淘汰回路 /
/// 任务表闸场景）。这里钉的是**不崩**：读不到注册表不是读到一个空世界。
#[tokio::test]
async fn without_a_visitor_nothing_is_in_any_gate() {
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    let decls = ToolDecls::of_metas(&symbio_core::tool_metas(&ctx).await);

    assert!(!decls.provides_assign_work("agent_run"));
    assert!(!decls.records_delegation_promise("agent_run"));
    assert!(!decls.owns_task_table("local/todo_write"));
}
