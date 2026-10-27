//! 工具自声明的行为名——声明闸与效果端口共用同一个读法。
//!
//! 钉三件事：
//!
//! - **各族只声明自己的工具**（`agent_run` 派活 + 立约、`todo_write` 任务表、别的
//!   工具一位都不声明）；
//! - **匹配口径**（线上全名先等值、再取末段；trim；空串与近似名不命中）；
//! - **声明随工具进子作用域**（改名投影只换名字、原样透传行为位）——闸门跟随
//!   工具，这是「换个会话绕不开淘汰」要的读法；
//! - **没有 visitor ⇒ 什么都不声明**（缺声明与漏声明同形，e2e 兜行为）。

use super::ToolDecls;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use symbio_core::{
    Capability, CapabilityMeta, CapabilityRow, ExecEnv, PluginError, PluginInvokeRequest,
    PluginInvokeRequestExt, PluginSimpleRequest, Registry,
};
use symbio_plugin_api::behaviors::{ASSIGN_WORK, DELEGATION_PROMISE, TASK_TABLE};
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
fn meta(name: &str, behaviors: &'static [&'static str]) -> CapabilityMeta {
    CapabilityMeta {
        name: name.to_string(),
        behaviors,
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
async fn factory_decls() -> ToolDecls {
    let ctx = ctx_with(vec![
        meta("agent_run", &[ASSIGN_WORK, DELEGATION_PROMISE]),
        meta("todo_write", &[TASK_TABLE]),
        meta("shell", &[]),
    ])
    .await;
    ToolDecls::of_metas(&symbio_core::tool_metas(&ctx).await)
}

// ==================== 各族只声明自己的工具 ====================

/// 三族声明互不串门：`agent_run` 两位、`todo_write` 一位、未声明的 `shell` 一位都没有。
#[tokio::test]
async fn each_family_declares_only_its_own_tool() {
    let decls = factory_decls().await;

    assert_eq!(
        decls.behaviors_of("agent_run"),
        &[ASSIGN_WORK, DELEGATION_PROMISE]
    );
    assert_eq!(decls.behaviors_of("local/todo_write"), &[TASK_TABLE]);
    assert!(
        decls.behaviors_of("local/shell").is_empty(),
        "未声明任何位的工具不在任何声明闸的管辖内"
    );
}

// ==================== 匹配口径 ====================

/// 线上全名先**等值**（空格不算名字的一部分）、再**取末段**；近似名不认。
#[tokio::test]
async fn matching_is_equality_then_last_segment() {
    let decls = factory_decls().await;

    assert_eq!(
        decls.behaviors_of(" agent_run "),
        &[ASSIGN_WORK, DELEGATION_PROMISE]
    );
    assert_eq!(decls.behaviors_of(" todo_write "), &[TASK_TABLE]);
    assert_eq!(decls.behaviors_of("local/todo_write"), &[TASK_TABLE]);
    assert_eq!(decls.behaviors_of("agent_scope/todo_write"), &[TASK_TABLE]);

    assert!(decls.behaviors_of("todo_writer").is_empty());
    assert!(decls.behaviors_of("local/todo_writer").is_empty());
    assert!(decls.behaviors_of("").is_empty());
}

/// 选实现时，**挂载点前缀不改变「这个工具声明了什么」**。
///
/// 反向自检：把 `behaviors_of` 改回只判等值 ⇒ 第二条断言红。那不是理论风险：
/// `todo_write` 的线上名一直是 `local/todo_write`，而注册表的键是短名——只判等值
/// 就等于「任务表效果永远不生效、写侧闸永远不命中」，且没有任何报错（J3 的静默形态）。
#[tokio::test]
async fn behavior_lookup_tolerates_a_mount_prefix() {
    let decls = factory_decls().await;

    assert_eq!(decls.behaviors_of("todo_write"), &[TASK_TABLE]);
    assert_eq!(decls.behaviors_of("local/todo_write"), &[TASK_TABLE]);
    assert!(
        decls.behaviors_of("mcp__srv__todo_write").is_empty(),
        "MCP 的同名工具不是同一个工具（短名不反演）"
    );
}

// ==================== 声明随工具进子作用域 ====================

/// 改名投影（子智能体的 `agent_<id>_<工具>`）只换名字、原样透传行为名，
/// 所以声明闸在子作用域**同样命中**——此前按字面量等值判，前缀名一个都不中。
///
/// 这不是扩面的口子，是「闸门跟随工具」的直读：淘汰判据本来就部署级
/// （扫全部会话的 WAL），没有理由在子会话里失明。
#[tokio::test]
async fn declarations_travel_into_scoped_projection_names() {
    let ctx = ctx_with(vec![
        meta(
            "agent_probe-agent_agent_run",
            &[ASSIGN_WORK, DELEGATION_PROMISE],
        ),
        meta("agent_probe-agent_todo_write", &[TASK_TABLE]),
    ])
    .await;
    let decls = ToolDecls::of_metas(&symbio_core::tool_metas(&ctx).await);

    assert_eq!(
        decls.behaviors_of("agent_probe-agent_agent_run"),
        &[ASSIGN_WORK, DELEGATION_PROMISE]
    );
    assert_eq!(
        decls.behaviors_of("agent_probe-agent_todo_write"),
        &[TASK_TABLE]
    );
}

// ==================== 没有 visitor ====================

/// 没有 visitor ⇒ 什么都读不到 ⇒ 没有声明闸命中、也没有效果可记。
///
/// 与「该声明的工具漏了声明」同一形态——都由 e2e 兜行为（T74 的淘汰回路 /
/// 任务表闸场景）。这里钉的是**不崩**：读不到注册表不是读到一个空世界。
#[tokio::test]
async fn without_a_visitor_nothing_is_declared() {
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    let decls = ToolDecls::of_metas(&symbio_core::tool_metas(&ctx).await);

    assert!(decls.behaviors_of("agent_run").is_empty());
    assert!(decls.behaviors_of("local/todo_write").is_empty());
}
