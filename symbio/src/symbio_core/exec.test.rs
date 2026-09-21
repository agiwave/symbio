//! `exec` 模块的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）：
//! `exec.rs` 只保留生产代码，测试全部放本文件。

use super::*;

use crate::symbio_core::schemas::session::session_chat_response::NodeOp;

struct RecordingWriter {
    ops: Arc<std::sync::Mutex<Vec<NodeOp>>>,
}

#[async_trait]
impl TranscriptWriter for RecordingWriter {
    async fn apply(&self, op: NodeOp) {
        self.ops.lock().unwrap().push(op);
    }
}

/// 进度计数：每次 emit 递增；静默出口恒为 0。
///
/// 执行层靠它区分「工具在推进」与「工具挂死」（空闲超时判据），因此
/// 「每一次 emit 都留下痕迹」是**判定正确性**的前提，不是可选优化。
#[tokio::test]
async fn sink_progress_counts_every_emit() {
    let ops = Arc::new(std::sync::Mutex::new(Vec::new()));
    let direct = EventSink::direct(Arc::new(RecordingWriter { ops }));
    let progress = direct.progress();
    assert_eq!(progress.emitted(), 0);

    direct
        .emit(NodeOp::Append {
            message_id: "m1".into(),
            delta: "a".into(),
        })
        .await;
    direct
        .emit(NodeOp::Append {
            message_id: "m1".into(),
            delta: "b".into(),
        })
        .await;
    assert_eq!(progress.emitted(), 2, "每次 emit 都必须留下进展痕迹");

    // 克隆（工具侧持有的那份）共享同一计数：执行层的读数必须与工具一致
    let tool_side = direct.progress();
    assert_eq!(tool_side.emitted(), 2);

    // 静默出口不发射 ⇒ 读数恒为 0：调用方无需分情形，退化为「总时长」口径
    assert_eq!(EventSink::silent().progress().emitted(), 0);
}

/// Direct 出口把操作原样交给写入点；Null 出口丢弃。
#[tokio::test]
async fn sink_direct_forwards_and_null_drops() {
    let ops = Arc::new(std::sync::Mutex::new(Vec::new()));
    let direct = EventSink::direct(Arc::new(RecordingWriter { ops: ops.clone() }));
    direct
        .emit(NodeOp::Append {
            message_id: "m1".into(),
            delta: "hi".into(),
        })
        .await;
    assert_eq!(ops.lock().unwrap().len(), 1);

    EventSink::silent()
        .emit(NodeOp::Append {
            message_id: "m1".into(),
            delta: "dropped".into(),
        })
        .await;
    assert_eq!(ops.lock().unwrap().len(), 1, "Null 出口不得写出任何东西");
}

/// abort 之后 is_aborted 为真、cancelled 立即返回（无轮询延迟）。
#[tokio::test]
async fn abort_signal_abort_is_immediate() {
    let sig = AbortSignal::new();
    assert!(!sig.is_aborted());
    sig.abort();
    assert!(sig.is_aborted());
    // 已中止时 cancelled 立即就绪
    tokio::time::timeout(std::time::Duration::from_millis(50), sig.cancelled())
        .await
        .expect("已中止的信号必须立即返回");
}

/// 克隆共享同一状态：一处 abort，另一处感知。
#[tokio::test]
async fn abort_signal_is_shared_across_clones() {
    let sig = AbortSignal::new();
    let waiter = sig.clone();
    let handle = tokio::spawn(async move { waiter.cancelled().await });
    sig.abort();
    tokio::time::timeout(std::time::Duration::from_millis(200), handle)
        .await
        .expect("克隆体必须被唤醒")
        .expect("等待任务不应 panic");
}

/// 未中止时 cancelled 不返回（保证不会误报中止）。
#[tokio::test]
async fn abort_signal_not_aborted_does_not_fire() {
    let sig = AbortSignal::new();
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), sig.cancelled())
            .await
            .is_err(),
        "未中止时 cancelled 不应就绪"
    );
}
