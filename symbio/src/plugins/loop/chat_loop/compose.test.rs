//! `session/chat_loop/compose.rs` 的单元测试 —— 对话面节点**进得了对话线**这一半。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。
//!
//! ## 为什么只剩投影认它这几条
//!
//! `dialog_node` 的构造器已上移 core `node_build`（[D-082](../../../../../docs/003-design/014-单环一表.md "D-082")
//! 裁决三），它的**纯形状**断言（落点 / 两个标记 / 理由码透传 / id 唯一）随构造器住进
//! `node_build.test.rs`。留在本文件的是另一半契约：**core 产出的这个节点，
//! session 的请求视图投影认不认**——`conversation_view` 不是 core 的地盘，这条边
//! 只有在这里才测得到。两条合起来才是完整的判据：形状对了但投影不认（或反之），
//! 前端「对话」面板都看不到它，且都没有错误信号。
//!
//! `apply_verdict` / `compose_text` 都要 `Arc<dyn PluginInvokeRequest>` 与容器路由，
//! 属 e2e 的射程（`e2e/cases/t22-compose.mjs`）——单测在这里重复一遍只会得到一份
//! 会漂移的假副本。

use super::*;
// `dialog_node` 本体已归 core，本文件因此不再经 `compose.rs` 的导入面拿到形状词表——
// 测试自己引（短名，不写全限定路径：S-014 的 role 直写棘轮按短名形状计数）。
use crate::symbio_core::chat_node::{MessageContent, NodeKind, NodeRole};

/// 取 `meta` 里某个键（`meta` 缺席时算没有这个键）。
fn meta_key(node: &ChatNode, key: &str) -> Option<serde_json::Value> {
    node.meta.as_ref()?.get(key).cloned()
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
/// "投影只认这个形状"。按"根级"切会让插件看不到助手上一轮的回答——
/// 前端「对话」面板看不到回答本身（回答在 Turn 组里）。
#[test]
fn a_turn_child_text_is_on_the_conversation_line() {
    let child = ChatNode {
        parent_id: Some("turn-1".to_string()),
        role: Some(NodeRole::Assistant),
        msg_type: Some(NodeKind::Body),
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
/// 与 `surface`（契约在 core `node_build.test.rs` 钉）同一条：改名不会报错，
/// 只会让前端静默失配。这一格的 owner 是 session（`compose` 侧自产，见实现处注释）。
#[test]
fn progress_reason_literal_is_the_wire_contract() {
    assert_eq!(REASON_PROGRESS, "progress");
}
