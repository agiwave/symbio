//! `symbio_core/schemas/session/node_build.rs` 的单元测试 —— 与实现同级分文件
//! （约定：`X.rs` + `X.test.rs`）。
//!
//! 两批用例随构造器自 session 上移（[D-082](../../../../docs/003-design/014-单环一表.md "D-082")
//! 裁决三，[plan/14 步 6](../../../../docs/003-design/014-单环一表.md "DE-014") 切片批）：
//!
//! - **对话面节点形状**（原 `chat_loop/compose.test.rs`）：`dialog_node` 承载的
//!   「落点 + 两个标记」——错了不会报错、只表现为界面不对，必须由断言钉住。
//!   其中依赖 `conversation_view` 投影的两条**留在 session 侧**（投影不是 core 的地盘）。
//! - **建树 flat 折叠**（原 `message_build.test.rs`）：`detach_if_flat` /
//! `flatten_whole_tree` / `turn_root_of` 的 相应批次 判据——`grouped`/`deep` 逐字
//!   零改动、flat 只脱本轮直接子节点、夹在工具卡内的子会话骨架绝不折叠。

use super::*;
use crate::symbio_core::chat_node::{CHAT_STYLE_TAG_RUN, CHAT_STYLE_TAG_TOOL};

// ── 对话面节点形状（dialog_node，原 compose.test.rs）────────────────────

/// 取 `meta` 里某个键（`meta` 缺席时算没有这个键）。
fn meta_key(node: &ChatNode, key: &str) -> Option<serde_json::Value> {
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

    assert_eq!(node.role, Some(NodeRole::Assistant));
    assert_eq!(node.msg_type, Some(NodeKind::Body));
    assert_eq!(node.status, Some(NodeStatus::Completed));
    assert_eq!(
        node.content.as_ref().map(|c| c.to_text()).as_deref(),
        Some("你好，我在。")
    );
    assert!(!node.id.is_empty(), "节点必须有 id（转写按 id 收敛）");
    assert!(node.timestamp.is_some(), "节点必须有时间戳（前端按它排序）");
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

/// 每次调用给出**新 id**：同一轮里 `Answered` 的答话与 `Escalate` 的首响若撞 id，
/// 转写会按 id 收敛成一条（后写的覆盖先写的），用户只看到一句话。
#[test]
fn each_node_gets_its_own_id() {
    let a = dialog_node("一", "greeting", false);
    let b = dialog_node("二", "greeting", false);

    assert_ne!(a.id, b.id, "两个节点不能共用 id");
}

// ── 建树策略 flat 折叠（相应批次，原 message_build.test.rs）──────────────

fn node(id: &str, parent: Option<&str>) -> ChatNode {
    ChatNode {
        id: id.to_string(),
        parent_id: parent.map(str::to_string),
        ..Default::default()
    }
}

fn run_node(id: &str) -> ChatNode {
    ChatNode {
        id: id.to_string(),
        msg_type: Some(NodeKind::Skeleton),
        ..Default::default()
    }
    .with_style_tag(CHAT_STYLE_TAG_RUN)
}

fn tool_node(id: &str, parent: &str) -> ChatNode {
    ChatNode {
        id: id.to_string(),
        parent_id: Some(parent.to_string()),
        msg_type: Some(NodeKind::Skeleton),
        ..Default::default()
    }
    .with_style_tag(CHAT_STYLE_TAG_TOOL)
}

#[test]
fn detach_if_flat_raises_direct_child_and_stamps_turn_root() {
    let mut n = node("body-1", Some("run-1"));
    detach_if_flat(&mut n, "run-1", PresentationTree::Flat);
    assert_eq!(n.parent_id, None, "flat：本轮直接子节点升根");
    assert_eq!(
        turn_root_of(&n),
        Some("run-1"),
        "升根后靠 turn_root 记号认回本轮骨架"
    );
}

#[test]
fn detach_if_flat_grouped_is_byte_for_byte_noop() {
    let mut n = node("body-1", Some("run-1"));
    n.meta = Some(serde_json::json!({ "style_tag": "reasoning" }));
    let before = serde_json::to_value(&n).unwrap();
    detach_if_flat(&mut n, "run-1", PresentationTree::Grouped);
    assert_eq!(
        serde_json::to_value(&n).unwrap(),
        before,
        "grouped 是今天行为：零改动"
    );
}

#[test]
fn detach_if_flat_ignores_child_not_under_this_root() {
    // 工具结果节点的父是 tool_call，不是本轮响应骨架——flat 也不该动它。
    let mut n = node("result-1", Some("tool-1"));
    detach_if_flat(&mut n, "run-1", PresentationTree::Flat);
    assert_eq!(n.parent_id.as_deref(), Some("tool-1"));
    assert_eq!(turn_root_of(&n), None);
}

#[test]
fn flatten_whole_tree_detaches_run_children_keeps_tool_results() {
    let mut tree = vec![
        run_node("run-1"),
        node("reasoning-1", Some("run-1")),
        node("text-1", Some("run-1")),
        tool_node("tool-1", "run-1"),
        node("result-1", Some("tool-1")),
    ];
    flatten_whole_tree(&mut tree, PresentationTree::Flat);
    // 骨架自身留在根；它的直接子节点（含工具调用骨架）升根 + 打记号。
    assert_eq!(tree[0].parent_id, None);
    for row in tree.iter().skip(1).take(3) {
        assert_eq!(row.parent_id, None, "本轮直接子节点升根");
        assert_eq!(turn_root_of(row), Some("run-1"));
    }
    // 工具结果挂在工具调用骨架下——与建树策略正交，flat 不碰。
    assert_eq!(tree[4].parent_id.as_deref(), Some("tool-1"));
    assert_eq!(turn_root_of(&tree[4]), None);
}

#[test]
fn flatten_whole_tree_grouped_is_noop() {
    let mut tree = vec![run_node("run-1"), node("text-1", Some("run-1"))];
    flatten_whole_tree(&mut tree, PresentationTree::Grouped);
    assert_eq!(
        tree[1].parent_id.as_deref(),
        Some("run-1"),
        "grouped 保持今天形状"
    );
    assert_eq!(turn_root_of(&tree[1]), None);
}

/// flat 折叠**不拆子会话**：转播桥把子会话顶层节点无条件挂到父工具卡下，那个
/// 转发来的 `Skeleton{run}` 仍 `is_run()`，但它的父是一张工具卡——必须跳过，否则
/// 子会话的孙节点会被错误升到会话根、拆散嵌套树。
#[test]
fn flatten_whole_tree_skips_run_skeleton_nested_in_tool_card() {
    let subrun = ChatNode {
        id: "subrun-1".to_string(),
        parent_id: Some("tool-1".to_string()),
        msg_type: Some(NodeKind::Skeleton),
        ..Default::default()
    }
    .with_style_tag(CHAT_STYLE_TAG_RUN);
    let mut tree = vec![
        run_node("run-1"),
        tool_node("tool-1", "run-1"),
        subrun,
        node("subtext-1", Some("subrun-1")),
    ];
    flatten_whole_tree(&mut tree, PresentationTree::Flat);
    // 顶层本轮骨架的直接子节点（工具卡）照常升根。
    assert_eq!(tree[1].parent_id, None, "工具卡升根");
    assert_eq!(turn_root_of(&tree[1]), Some("run-1"));
    // 子会话的 run 骨架仍挂在工具卡下（未被当作折叠根）。
    assert_eq!(
        tree[2].parent_id.as_deref(),
        Some("tool-1"),
        "夹在工具卡内的子会话骨架不折叠"
    );
    // 其孙节点保持嵌套，绝不升根。
    assert_eq!(
        tree[3].parent_id.as_deref(),
        Some("subrun-1"),
        "子会话孙节点不被错误升根"
    );
    assert_eq!(turn_root_of(&tree[3]), None);
}

/// `deep` 与 `grouped` 逐字相同：读侧投影都不升根，区别只在前端如何展开嵌套树。
#[test]
fn flatten_whole_tree_deep_is_noop() {
    let mut tree = vec![run_node("run-1"), node("text-1", Some("run-1"))];
    let before = serde_json::to_value(&tree).unwrap();
    flatten_whole_tree(&mut tree, PresentationTree::Deep);
    assert_eq!(
        serde_json::to_value(&tree).unwrap(),
        before,
        "deep 不做任何升根折叠"
    );
    assert_eq!(tree[1].parent_id.as_deref(), Some("run-1"));
    assert_eq!(turn_root_of(&tree[1]), None);
}

#[test]
fn flatten_whole_tree_is_idempotent() {
    let mut once = vec![run_node("run-1"), node("text-1", Some("run-1"))];
    flatten_whole_tree(&mut once, PresentationTree::Flat);
    let mut twice = once.clone();
    flatten_whole_tree(&mut twice, PresentationTree::Flat);
    assert_eq!(
        serde_json::to_value(&once).unwrap(),
        serde_json::to_value(&twice).unwrap(),
        "投影叠在已升根的树上不二次变形"
    );
}
