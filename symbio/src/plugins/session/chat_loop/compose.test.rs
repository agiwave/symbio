//! `session/chat_loop/compose.rs` 的单元测试 —— 对话面节点的**形状**与两个标记。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。
//!
//! ## 为什么只测 `dialog_node`
//!
//! `apply_verdict` / `compose_text` 都要 `Arc<dyn PluginInvokeRequest>` 与容器路由，
//! 属 e2e 的射程（`e2e/cases/t22-reply.mjs`）——单测在这里重复一遍只会得到一份
//! 会漂移的假副本。而 `dialog_node` 是**纯函数**，它承载本文件里唯一"错了不会报错、
//! 只表现为界面不对"的决定：
//!
//! - **落点**（根级 or 挂在 Turn 之下）；
//! - **两个标记**（`surface` / `exclude_from_context`）各自的取值。
//!
//! 这两件事错了都没有错误信号：节点照样落库、照样上线，只是前端「对话」面板
//! 看不到它，或者模型收到连续两条 `assistant`（部分协议直接 400）。因此它们
//! 必须被断言钉住。

use super::*;

/// 取 `meta` 里某个键（`meta` 缺席时算没有这个键）。
fn meta_key(node: &ChatMessage, key: &str) -> Option<serde_json::Value> {
    node.meta.as_ref()?.get(key).cloned()
}

/// 一个 assistant 的根级文本节点：身份三件套齐备（角色 / 类型 / 终态）。
///
/// 三个字段分别管三件事：`role` 决定前端气泡朝向，`msg_type` 决定用哪种渲染器，
/// `status` 决定它还转不转圈。任何一个缺省都会被渲染层当成"未知"降级处理——
/// 不报错，只是长得不对。
#[test]
fn dialog_node_is_a_completed_assistant_text_node() {
    let node = dialog_node("你好，我在。", "greeting", false);

    assert_eq!(node.role, Some(MessageRole::Assistant));
    assert_eq!(node.msg_type, Some(MessageType::Text));
    assert_eq!(node.status, Some(MessageStatus::Completed));
    assert_eq!(
        node.content.as_ref().map(|c| c.to_text()).as_deref(),
        Some("你好，我在。")
    );
    assert!(!node.id.is_empty(), "节点必须有 id（转写按 id 收敛）");
    assert!(node.timestamp.is_some(), "节点必须有时间戳（前端按它排序）");
}

/// **落点判据**：根级（`parent_id = None`）——与用户消息互为兄弟。
///
/// 这条断言刻意不写"`parent_id` 是 `None`"，而是**用投影函数反过来验证**：
/// 「落点正确」的定义就是「对话线认它」。写成字面量断言的话，哪天投影规则改了，
/// 这里仍然绿——而实际效果（前端「对话」面板看不看得见）已经坏了。
#[test]
fn dialog_node_lands_on_the_conversation_line() {
    let node = dialog_node("好，我来处理。", "needs_work", true);

    let line = conversation_view(std::slice::from_ref(&node), 0);
    assert_eq!(
        line.len(),
        1,
        "对话面节点必须被 conversation_view 认出来——否则前端「对话」面板看不到它，\
         且下一轮 reply 不知道上一轮说过什么（措辞会重复或断裂）"
    );
    assert_eq!(line[0].id, node.id);
}

/// 挂在 Turn 之下的节点**不是**对话线的一部分——反向钉住上一条。
///
/// 没有这条，`dialog_node_lands_on_the_conversation_line` 可能因为"投影根本不过滤"
/// 而通过。两个断言合起来才说明「根级」这个选择是**有判别力的**。
#[test]
fn a_turn_child_text_is_not_on_the_conversation_line() {
    let child = ChatMessage {
        parent_id: Some("turn-1".to_string()),
        role: Some(MessageRole::Assistant),
        msg_type: Some(MessageType::Text),
        content: Some(MessageContent::Text("回复正文".to_string())),
        ..Default::default()
    };

    assert!(
        conversation_view(std::slice::from_ref(&child), 0).is_empty(),
        "Turn 的子文本节点是回复正文，属工作线——它不该出现在对话线投影里"
    );
}

/// `Answered` 的答话：`surface = "reply"`，且**不设** `exclude_from_context`。
///
/// "不设"而不是"设成 `false`"：缺席是**缺省即进请求包**，而 `false` 是一个显式
/// 取值。两者在今天等价，但前者让"这个键只有一种含义"成立——见下一条断言。
#[test]
fn answered_answer_is_not_excluded_from_context() {
    let node = dialog_node("你好，我在。", "greeting", false);

    assert_eq!(meta_key(&node, "surface"), Some(serde_json::json!("reply")));
    assert_eq!(
        meta_key(&node, "exclude_from_context"),
        None,
        "Answered 的答话就是这一轮的答复，必须进请求包（它是对话内容）"
    );
}

/// `Escalate` 的首响：同一个 `surface`，但**带** `exclude_from_context = true`。
///
/// 两条理由（缺一不可）：① 它是面向用户的**界面文本**，不是"模型的对话历史"；
/// ② 它紧跟用户消息，若进请求包，线上会出现连续两条 `assistant`——部分协议
/// （Anthropic 一类要求角色交替）直接 400。而 400 是**远端**的错误，本地看不出
/// 是哪个节点引起的。
#[test]
fn escalate_opening_is_excluded_from_context() {
    let node = dialog_node("好，我来处理。", "needs_work", true);

    assert_eq!(meta_key(&node, "surface"), Some(serde_json::json!("reply")));
    assert_eq!(
        meta_key(&node, "exclude_from_context"),
        Some(serde_json::json!(true)),
        "Escalate 的首响是界面开场白，必须被请求视图剔除"
    );
}

/// 两个标记回答**两个不同的问题**，因此同一个 `surface` 下可以有相反的
/// `exclude_from_context`——本条与上两条一起说明"它们没有合并"。
#[test]
fn the_two_markers_are_independent() {
    let answered = dialog_node("你好，我在。", "greeting", false);
    let escalated = dialog_node("好，我来处理。", "needs_work", true);

    assert_eq!(
        meta_key(&answered, "surface"),
        meta_key(&escalated, "surface"),
        "两者都由措辞能力产出 ⇒ surface 相同"
    );
    assert_ne!(
        meta_key(&answered, "exclude_from_context"),
        meta_key(&escalated, "exclude_from_context"),
        "两者对「模型该不该看到」的回答相反 ⇒ 该标记必须能不同"
    );
}

/// 理由码**原样**带上，不做映射、不校验取值。
///
/// 它是**数据**（J1）：新增一类理由只加一行码表，两侧不共享常量（`reply` 侧持有
/// 抄本，未知码走通用模板）。本函数若在这里做白名单，等于把"可扩展"变成"改 core"。
#[test]
fn reason_code_is_carried_verbatim() {
    for reason in ["greeting", "from_context", "某个本仓还没定义的理由码"] {
        let node = dialog_node("话", reason, false);
        assert_eq!(
            meta_key(&node, "reason"),
            Some(serde_json::json!(reason)),
            "理由码必须原样透传（它是数据，不是闭集）"
        );
    }
}

/// 正文**原样**写入：不 trim、不加前后缀。
///
/// 措辞的加工在 `reply` 侧（提示词约束 + 模板），这里若再加工一次，就会出现
/// "两处都在管正文长什么样"——而它们会各演化一次。本函数的职责只有"落点 + 标记"。
#[test]
fn text_is_written_verbatim() {
    let raw = "  前面有空格，后面也有。  ";
    let node = dialog_node(raw, "ack", false);

    assert_eq!(
        node.content.as_ref().map(|c| c.to_text()).as_deref(),
        Some(raw)
    );
}

/// `surface` 的取值是**跨端契约**（前端按它区分"这句话是对话面产出的"）。
///
/// 钉住字面量：改名会让前端的过滤静默失配——节点照样上线，只是不再被当成对话面
/// 节点渲染（表现为"助手说的话跑进了工作面板"），没有任何错误信号。
#[test]
fn surface_literal_is_the_wire_contract() {
    assert_eq!(SURFACE_REPLY, "reply");
}

/// 每次调用给出**新 id**：同一轮里 `Answered` 的答话与 `Escalate` 的首响若撞 id，
/// 转写会按 id 收敛成一条（后写的覆盖先写的），用户只看到一句话。
#[test]
fn each_node_gets_its_own_id() {
    let a = dialog_node("一", "greeting", false);
    let b = dialog_node("二", "greeting", false);

    assert_ne!(a.id, b.id, "两个节点不能共用 id");
}
