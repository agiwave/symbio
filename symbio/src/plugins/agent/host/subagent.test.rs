//! `symbio/src/plugins/agent/host/subagent.rs` 的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。
//!
//! 转播桥的循环体本身要 provider / sink / abort 信号才跑得动，端到端覆盖在
//! `e2e/cases/t15-subagent-inbox.mjs`；这里钉的是**编排之外的四组判据**——
//! 「本轮是否结束」（[`turn_end_of`]：两个读数来源共用一份判据）、地址形状（[`is_message_item_addr`]）、
//! 冒泡识别（[`pending_bubble_of`]）、消息过滤（[`message_fate`]）与节点改写
//! （[`relabel_for_parent`]，含「主体改写排在角色改写之后」那条顺序依赖）。
//! 拆出来才有单测可言：判据住在 238 行的循环体里时，「审批冒泡的判据是什么」无处引用。

use super::*;

use crate::symbio_core::principal::{Principal, PrincipalKind};
use crate::symbio_core::{VdfsAccess, VDFS_STATUS_ACTIVE, VDFS_STATUS_FAILED};

fn session_node(status: &str, error: Option<&str>) -> VdfsNode {
    let mut n = VdfsNode::dir("sid", "子会话", VdfsAccess::READ);
    n.status = status.to_string();
    if let Some(e) = error {
        n.attributes.insert("error".to_string(), json!(e));
    }
    n
}

/// 仍在 `working` ⇒ `None`（继续等）。这一格判错会表现为「子会话没跑完就收尾」。
#[test]
fn working_session_is_not_an_end() {
    assert_eq!(turn_end_of(&session_node(VDFS_STATUS_WORKING, None)), None);
}

/// 离开 `working` ⇒ `Some(_)`；无 `error` 属性时是正常收尾 `Some(None)`。
#[test]
fn finished_session_ends_without_error() {
    assert_eq!(
        turn_end_of(&session_node(VDFS_STATUS_ACTIVE, None)),
        Some(None),
        "会话节点离开 working 就是本轮结束的唯一判据（与 telegram / cli 同一条）"
    );
}

/// 带 `error` 属性收尾 ⇒ 错误随判据一起带出（本桥的结局优先级：错误 > 待审批 > 文本）。
#[test]
fn failed_session_carries_its_error() {
    assert_eq!(
        turn_end_of(&session_node(VDFS_STATUS_FAILED, Some("模型不可用"))),
        Some(Some("模型不可用".to_string())),
    );
}

// ── 帧构造：两种身份形状各一个入口 ──────────────────────────────────

/// **存量形状**的帧：身份只写 `role` 那一格（子会话回读出来的节点正是这一形状，
/// `node_role()` 在主体缺席时按这一格兜底——D-147 之后主体才是先手）。
///
/// 本文件的判据有一半读的就是这一格（过滤折角色、改写顺序），换成 `principal` 就测不到；
/// 所以把它集中成**一个**构造点，S-014 的那笔债也只长这一格。
fn legacy_frame(id: &str, role: NodeRole) -> ChatNode {
    let mut node = ChatNode {
        id: id.into(),
        ..Default::default()
    };
    // grep-audit-allow S-014: 判据对象就是存量 role 格（node_role() 的这条读法），全文件集中在这一处写
    node.role = Some(role);
    node
}

/// **新形状**的帧：`role` 格为空，族只由 `principal.kind` 承载（`node_role()` 折出来）。
fn principal_frame(id: &str, kind: PrincipalKind, who: &str) -> ChatNode {
    ChatNode {
        id: id.into(),
        principal: Some(Principal::new(kind, who)),
        ..Default::default()
    }
}

// ── 地址形状：变更落点是不是「某条消息自身」 ──────────────────────────

/// 只认 `<前缀>/<项 id>`：集合目录本身、更深一层、别的集合段、别的会话一律不认。
///
/// 这条判据错一格的代价不对称：多认 ⇒ 别人的节点（记忆 / 工作目录 / 子会话）被当消息
/// 转播进父视图；少认 ⇒ 流式正文收不到。所以四个反例都要钉住。
#[test]
fn only_message_item_addresses_are_relayed() {
    let prefix = "session/child-1/message/";
    assert!(is_message_item_addr(prefix, "session/child-1/message/m-9"));
    assert!(
        !is_message_item_addr(prefix, "session/child-1/message"),
        "集合目录本身不是消息"
    );
    assert!(
        !is_message_item_addr(prefix, "session/child-1/message/m-9/attachments/a"),
        "更深一层不是消息"
    );
    assert!(
        !is_message_item_addr(prefix, "session/child-1/memory/m-9"),
        "别的集合段不是消息"
    );
    assert!(
        !is_message_item_addr(prefix, "session/child-2/message/m-9"),
        "别的会话的消息与本桥无关"
    );
    assert!(
        !is_message_item_addr(prefix, "session/child-1/inbox"),
        "输入面不是转播面"
    );
}

// ── 冒泡识别：审批与提问 ────────────────────────────────────────────

/// `Interaction` + `WaitingUserAction` 才算冒泡：缺一条都不弹卡。
///
/// 前者缺 ⇒ 普通正文被当成待回应；后者缺 ⇒ **过去**的交互被重新弹一次，
/// 用户会看到一张回答过的卡。
#[test]
fn waiting_interaction_bubbles_as_a_resume_payload() {
    let mut card = principal_frame("m-7", PrincipalKind::Tool, "shell");
    card.msg_type = Some(NodeKind::Interaction);
    card.status = Some(NodeStatus::WaitingUserAction);
    card.meta = Some(json!({ "failure_kind": "needs_interaction" }));
    let bubble = pending_bubble_of(&card, "要把这个文件删掉吗？", Some("researcher"), "child-1")
        .expect("等待用户动作的交互必须冒泡");
    let RelayOutcome::Pending {
        text,
        prompt,
        failure_kind,
    } = bubble
    else {
        panic!("冒泡只能是 Pending 变体")
    };
    assert_eq!(text, "要把这个文件删掉吗？", "卡片正文是冒泡前累积的正文");
    assert_eq!(
        failure_kind,
        crate::symbio_core::failure_kind::NEEDS_INTERACTION,
        "子会话给的 kind 原样带上（前端按它分审批 UI 与提问 UI）"
    );
    // 续跑参数是**本工具**的：agent_id / session_id / target_id 三样缺一不可 resume。
    assert_eq!(prompt["tool_name"], json!("agent_run"));
    assert_eq!(prompt["args"]["agent_id"], json!("researcher"));
    assert_eq!(prompt["args"]["session_id"], json!("child-1"));
    assert_eq!(prompt["args"]["target_id"], json!("m-7"));
}

/// 非冒泡帧一律 `None`：正正文（同状态）、已完成的历史交互、以及 kind 缺失时不越权猜。
#[test]
fn ordinary_frames_do_not_bubble() {
    let body = ChatNode {
        id: "m-1".into(),
        msg_type: Some(NodeKind::Body),
        status: Some(NodeStatus::WaitingUserAction),
        ..Default::default()
    };
    assert!(pending_bubble_of(&body, "", None, "child-1").is_none());

    let settled = ChatNode {
        id: "m-2".into(),
        msg_type: Some(NodeKind::Interaction),
        status: Some(NodeStatus::Completed),
        ..Default::default()
    };
    assert!(pending_bubble_of(&settled, "", None, "child-1").is_none());

    // `meta` 里没有 kind ⇒ 兜底成审批（保守：宁可让用户批一次，不能默默放行）。
    let untyped = ChatNode {
        id: "m-3".into(),
        msg_type: Some(NodeKind::Interaction),
        status: Some(NodeStatus::WaitingUserAction),
        ..Default::default()
    };
    let bubble = pending_bubble_of(&untyped, "", None, "child-1").expect("仍是待用户动作");
    let RelayOutcome::Pending {
        failure_kind,
        prompt,
        text,
    } = bubble
    else {
        panic!()
    };
    assert_eq!(
        failure_kind,
        crate::symbio_core::failure_kind::NEEDS_APPROVAL
    );
    assert_eq!(
        prompt["args"]["agent_id"],
        Value::Null,
        "委托时可无 agent_id"
    );
    assert_eq!(text, "", "没有累积正文就是空串，不是 null");
}

// ── 消息过滤：这一帧落成父视图里的什么 ──────────────────────────────

/// 删除语义优先于角色：已删的节点不再落节点，也不该被读成「不透传」。
///
/// 两个反例各自对应一种漂法：user 节点漏进父视图 ⇒ 前端拿到一张编辑必败的卡；
/// 删除被折成不透传 ⇒ 父视图里那条节点永远删不掉。
#[test]
fn removal_and_user_frames_take_their_own_paths() {
    let mut deleted = legacy_frame("m-1", NodeRole::User);
    deleted.status = Some(NodeStatus::Removed);
    assert!(
        matches!(message_fate(&deleted), MessageFate::Removal),
        "删除判据不看角色"
    );
    assert!(
        matches!(
            message_fate(&legacy_frame("m-2", NodeRole::User)),
            MessageFate::Withheld
        ),
        "委托 prompt 不透传"
    );
    for role in [NodeRole::Assistant, NodeRole::Tool, NodeRole::System] {
        assert!(
            matches!(
                message_fate(&legacy_frame("m-3", role.clone())),
                MessageFate::Relay
            ),
            "{role:?} 的节点要转播进父视图"
        );
    }
    // 窄增量帧没有身份格 ⇒ 按帧只能判 Relay（真正的过滤由调用点拿**已知身份**再判一次）。
    let mut delta = ChatNode {
        id: "m-4".into(),
        ..Default::default()
    };
    delta.delta = Some("片段".into());
    assert!(matches!(message_fate(&delta), MessageFate::Relay));
    // 旧格为空、族由主体承载 ⇒ 折出来的 `User` 同样不透传（漏这一条等于按 `role` 格过滤）。
    assert!(
        matches!(
            message_fate(&principal_frame("m-5", PrincipalKind::Person, "dynote365")),
            MessageFate::Withheld
        ),
        "过滤走 node_role() 这条唯一读入口，不走残留的 role 格"
    );
}

// ── 节点改写：锚定 / 角色 / 主体，以及它们之间的顺序 ──────────────────

/// 顶层助手正文 = 本工具的响应：锚到 ToolCall 之下、族改成 `Tool`、主体跟着改。
#[test]
fn relabel_anchors_top_level_assistant_as_tool_response() {
    let mut node = legacy_frame("m-1", NodeRole::Assistant);
    node.principal = Some(Principal::new(PrincipalKind::Agent, "sub-agent"));
    node.content = Some(MessageContent::Text("跑完了".into()));
    relabel_for_parent(&mut node, "call-9");
    assert_eq!(
        node.parent_id.as_deref(),
        Some("call-9"),
        "顶层挂到本工具调用之下"
    );
    assert_eq!(
        node.node_role(),
        Some(NodeRole::Tool),
        "分型结构 ToolCall(Assistant) → Turn(Tool)，且 flatten 的 find_tool_result 认的是折叠后的族"
    );
    let principal = node.principal.expect("主体就是身份的那一份写");
    assert_eq!(principal.kind, PrincipalKind::Tool);
    assert_eq!(
        principal.id, "agent/run",
        "族归工具自己（`agent_run` 的合成地址），否则 visible_to 当它是子 agent 的第三方私有"
    );
    assert_eq!(
        node.content.as_ref().map(|c| c.to_text()),
        Some("跑完了".to_string()),
        "改写只动身份三格，正文原样"
    );
}

/// **身份只有一份写**（D-147）：改写主体后折叠当场读回 `Tool`，不再往 `role` 格补
/// 第二份——旧版本两格都写、且**顺序写反就会让 `role` 格永远补不上**（折叠读的是
/// 残留格，先写主体的话第二次读已是 `Tool`，改写分支当场跳过）。
///
/// 这条用例钉住的是「顺序不再是判据」：起始节点只有主体（族靠主体折出）、
/// 改写后 `role` 格仍是空，而折叠结果已经是 `Tool`。
#[test]
fn relabel_writes_only_the_principal_and_folds_to_tool() {
    let mut folded = principal_frame("m-2", PrincipalKind::Agent, "sub-agent");
    assert_eq!(
        folded.node_role(),
        Some(NodeRole::Assistant),
        "前置条件：这一格的 Assistant 是折出来的（role 为空、族靠主体）"
    );
    relabel_for_parent(&mut folded, "call-9");
    assert_eq!(
        folded.legacy_role(),
        None,
        "改写不碰残留格——补第二份才有「顺序写反就永远补不上」那个坑"
    );
    assert_eq!(
        folded.node_role(),
        Some(NodeRole::Tool),
        "折叠以主体为先：改完主体就当场是 Tool，与写入顺序无关"
    );
    assert_eq!(
        folded.principal.as_ref().map(|p| p.kind),
        Some(PrincipalKind::Tool)
    );
}

/// 已带父锚点的节点（子会话内部的父子结构）**整格不动**；非助手节点只补锚点。
///
/// 反例代价：改写子会话内部的层级会把子会话自己的结构拍平到工具调用之下；
/// 而给 `Tool` / `System` 节点盖 `agent_run` 主体等于伪造来源。
#[test]
fn relabel_leaves_nested_and_non_assistant_nodes_alone() {
    let mut nested = legacy_frame("m-3", NodeRole::Assistant);
    nested.parent_id = Some("m-2".into());
    relabel_for_parent(&mut nested, "call-9");
    assert_eq!(
        nested.parent_id.as_deref(),
        Some("m-2"),
        "嵌套结构原样带过来"
    );
    assert_eq!(
        nested.legacy_role(),
        Some(NodeRole::Assistant),
        "不改写子会话内部的层级"
    );

    let mut tool = legacy_frame("m-4", NodeRole::Tool);
    relabel_for_parent(&mut tool, "call-9");
    assert_eq!(
        tool.parent_id.as_deref(),
        Some("call-9"),
        "工具响应也要挂到本调用之下"
    );
    assert_eq!(tool.legacy_role(), Some(NodeRole::Tool));
    assert!(tool.principal.is_none(), "不是助手正文就不伪造主体");
}
