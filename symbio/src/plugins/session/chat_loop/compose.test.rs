//! `session/chat_loop/compose.rs` 的单元测试 —— 对话面节点的**形状**与两个标记。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。
//!
//! ## 为什么只测 `dialog_node`
//!
//! `apply_verdict` / `compose_text` 都要 `Arc<dyn PluginInvokeRequest>` 与容器路由，
//! 属 e2e 的射程（`e2e/cases/t22-compose.mjs`）——单测在这里重复一遍只会得到一份
//! 会漂移的假副本。而 `dialog_node` 是**纯函数**，它承载本文件里唯一"错了不会报错、
//! 只表现为界面不对"的决定：
//!
//! - **落点**（根级 or 挂在 Turn 之下）；
//! - **两个标记**（`surface` / `exclude_from_context`）各自的取值。
//!
//! 这两件事错了都没有错误信号：节点照样落库、照样上线，只是前端「对话」面板
//! 看不到它，或者模型收到连续两条 `assistant`（部分协议直接 400）。因此它们
//! 必须被断言钉住。
//!
//! 三种产物（`Answered` 答话 / `Escalate` 首响 / `Report` 汇报）各有一条形状用例：
//! 它们的 `surface` 相同、`exclude_from_context` 相反、理由码不同——分开验才说明
//! 两个标记没有被合并成一个。

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
/// 两个断言各钉一件事，缺一不可：
///
/// - **根级**：位置**不再是**对话线的判据（判据只有角色与类型，见
///   `conversation_view` 的模块文档），因此"落在根级"必须在这里字面钉住。
///   挂到 Turn 之下，首响就变成"工作的一轮"，前端「对话」面板看到的不再是一个
///   独立的节点。
/// - **对话线认它**：它是一句 assistant 文本，必须被投影收进去——否则下一轮
///   `compose` 不知道上一轮说过什么（措辞会重复或断裂），前端「对话」面板也看不到它。
#[test]
fn dialog_node_lands_at_the_root_and_on_the_conversation_line() {
    let node = dialog_node("好，我来处理。", "needs_work", true);

    assert!(
        node.parent_id.is_none(),
        "对话面节点必须落在根级——它是一句独立的话，不是某一轮工作的一部分"
    );
    let line = conversation_view(std::slice::from_ref(&node), 0);
    assert_eq!(
        line.len(),
        1,
        "对话面节点必须被 conversation_view 认出来——否则前端「对话」面板看不到它，\
         且下一轮 compose 不知道上一轮说过什么（措辞会重复或断裂）"
    );
    assert_eq!(line[0].id, node.id);
}

/// 助手正文（`Turn` 的子 `Text`）**在**对话线上——判据不看位置。
///
/// 与上一条合起来说明「根级」是**语义选择**（首响是一句独立的话），而不是
/// "投影只认这个形状"。曾经按"根级"切，代价是插件看不到助手上一轮的回答、
/// 前端「对话」面板看不到回答本身（回答在 Turn 组里）。
#[test]
fn a_turn_child_text_is_on_the_conversation_line() {
    let child = ChatMessage {
        parent_id: Some("turn-1".to_string()),
        role: Some(MessageRole::Assistant),
        msg_type: Some(MessageType::Text),
        content: Some(MessageContent::Text("回复正文".to_string())),
        ..Default::default()
    };

    let line = conversation_view(std::slice::from_ref(&child), 0);
    assert_eq!(
        line.len(),
        1,
        "Turn 的子文本节点是回复正文——它就是用户读到的那段回答，必须在对话线上"
    );
    assert_eq!(line[0].id, child.id);
}

/// `Answered` 的答话：`surface = "compose"`，且**不设** `exclude_from_context`。
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
/// 它是**数据**（J1）：新增一类理由只加一行码表，两侧不共享常量（`compose` 侧持有
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
/// 措辞的加工在 `compose` 侧（提示词约束 + 模板），这里若再加工一次，就会出现
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

/// `Report` 的汇报：与首响同一形状（根级 + 剔除），理由码是 `progress`。
///
/// 三条各自独立的理由：**根级**（对话线才认它——否则前端「对话」面板看不到，
/// 而"中途汇报"的全部意义就是在对话里说一句）；**剔除**（进度是给用户看的，
/// 不是模型的对话内容——它紧跟在模型自己的工具轮之后，进包会白白占用上下文）；
/// **理由码**（前端据此与首响区分渲染）。
///
/// 它验的是"给定这三个参数，节点长对了"；调用点是否**真的**传了这三个参数由 e2e
/// （`t23-progress-report.mjs`）在真实边界上验——两者缺一，另一条都可能因为走错
/// 路径而恰好通过。
#[test]
fn report_node_is_excluded_and_on_the_conversation_line() {
    let node = dialog_node(
        "已经完成 3 轮工具调用，用时约 2 分钟，还在继续。",
        REASON_PROGRESS,
        true,
    );

    assert_eq!(meta_key(&node, "surface"), Some(serde_json::json!("reply")));
    assert_eq!(
        meta_key(&node, "reason"),
        Some(serde_json::json!("progress"))
    );
    assert_eq!(
        meta_key(&node, "exclude_from_context"),
        Some(serde_json::json!(true)),
        "汇报是给用户看的界面文本，必须被请求视图剔除"
    );
    assert!(node.parent_id.is_none(), "汇报挂在根级（对话线）");
    assert_eq!(
        conversation_view(std::slice::from_ref(&node), 0).len(),
        1,
        "汇报必须被对话线投影认出来——否则它落在工作面板里，用户看不到"
    );
}

/// `Report` 的理由码是**跨端契约**（前端按它区分"这是汇报不是首响"）。
///
/// 与 `surface` 同一条：改名不会报错，只会让前端静默失配。
#[test]
fn progress_reason_literal_is_the_wire_contract() {
    assert_eq!(REASON_PROGRESS, "progress");
}

/// 每次调用给出**新 id**：同一轮里 `Answered` 的答话与 `Escalate` 的首响若撞 id，
/// 转写会按 id 收敛成一条（后写的覆盖先写的），用户只看到一句话。
#[test]
fn each_node_gets_its_own_id() {
    let a = dialog_node("一", "greeting", false);
    let b = dialog_node("二", "greeting", false);

    assert_ne!(a.id, b.id, "两个节点不能共用 id");
}
