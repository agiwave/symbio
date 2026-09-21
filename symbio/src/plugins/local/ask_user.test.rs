//! `ask_user` 模块的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）：
//! `ask_user.rs` 只保留生产代码，测试全部放本文件。

use super::*;
use crate::symbio_core::SimpleRequest;

/// 构造带 payload 与运行模式的请求上下文。
fn ctx_with(args: Value, mode: &str) -> Arc<dyn InvokeRequest> {
    let req = SimpleRequest::new(None, None);
    req.set(crate::symbio_core::MODE, mode.to_string());
    // payload 以原生 JSON 存储：typed PAYLOAD 键带 deprecated 标记，用 set_raw 规避告警
    req.set_raw("payload", Arc::new(args));
    Arc::new(req)
}

fn ask_args() -> Value {
    json!({
        "question": "选哪个？",
        "options": [{ "label": "A" }, { "label": "B" }],
    })
}

/// 交互模式下 `ask_user` **返回意图与载荷，而不是一条通道**。
///
/// 这是本批的要点：工具不再自建 Session 通道、不再自造节点 id。返回的
/// `failure_kind` 是编排层收口为 `WaitingUserAction` 的**唯一**判据，
/// `prompt` 是提问卡的唯一内容来源。
#[tokio::test]
async fn interactive_mode_returns_pending_intent_not_a_channel() {
    let tool = AskUserTool;
    let ctx = ctx_with(ask_args(), "interactive");
    let env = ExecEnv::from_request(&*ctx);
    let data = tool
        .execute(ask_args(), &env, ctx)
        .await
        .expect("交互模式应成功返回");

    // 返回的是**工具结果本身**（意图 + 载荷），不再是 `PluginPayload` 多态载荷，
    // 更不得是 Session 通道。
    assert_eq!(
        data["failure_kind"],
        json!(crate::symbio_core::failure_kind::NEEDS_INTERACTION),
        "编排层凭 failure_kind 判定本轮结束于等待用户"
    );
    assert!(
        data.get("prompt").is_some(),
        "必须带上提问卡载荷——节点由编排层构造，它只有这一个内容来源"
    );
    assert!(
        data.get("id").is_none(),
        "工具不得自带节点身份：id 归编排层，自带就是「同一节点两个 id」事故的起点"
    );
}

/// 自动模式：无人可回答，必须返回**可继续的错误**而不是等待。
///
/// `tool_unavailable` 是信息性标记，`is_pending` 必须为 false——否则无人值守会话
/// 会等一个永远不会来的回答，永久挂起（这正是 pending 闭集要挡住的失效模式）。
#[tokio::test]
async fn auto_mode_returns_continuable_error_not_pending() {
    let tool = AskUserTool;
    let ctx = ctx_with(ask_args(), "auto");
    let env = ExecEnv::from_request(&*ctx);
    let data = tool
        .execute(ask_args(), &env, ctx)
        .await
        .expect("自动模式应返回可继续的错误");

    let kind = data["failure_kind"].as_str().unwrap_or_default();
    assert_eq!(kind, crate::symbio_core::failure_kind::TOOL_UNAVAILABLE);
    assert!(
        !crate::symbio_core::failure_kind::is_pending(kind),
        "自动模式不得判为等待用户"
    );
    assert!(data.get("prompt").is_none(), "自动模式不产卡片载荷");
}

/// 选项数不足 2 个时拒绝（提问卡的选项契约），且错误是 `ValidationError`。
#[tokio::test]
async fn too_few_options_is_rejected() {
    let tool = AskUserTool;
    let ctx = ctx_with(
        json!({ "question": "选哪个？", "options": [{ "label": "只有一个" }] }),
        "interactive",
    );
    let env = ExecEnv::from_request(&*ctx);
    assert!(
        tool.execute(
            json!({ "question": "选哪个？", "options": [{ "label": "只有一个" }] }),
            &env,
            ctx
        )
        .await
        .is_err(),
        "少于 2 个选项应被拒绝，而不是产出一张没法选的卡"
    );
}
