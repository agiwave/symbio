//! `symbio/src/plugins/agent/host/subagent.rs` 的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。
//!
//! 转播桥的循环体本身要 provider / sink / abort 信号才跑得动，端到端覆盖在
//! `e2e/cases/t15-subagent-inbox.mjs`；这里只钉那颗**判据**——「本轮是否结束」
//! 有两个读数来源（会话变更帧自带的载荷、背压重同步后的回读），判据只许有一份。

use super::*;

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
