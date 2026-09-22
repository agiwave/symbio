//! `exec` 模块的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）：
//! `exec.rs` 只保留生产代码，测试全部放本文件。

use super::*;

use crate::symbio_core::schemas::session::chat_message::ChatMessage;

struct RecordingWriter {
    ops: Arc<std::sync::Mutex<Vec<ChatMessage>>>,
}

#[async_trait]
impl TranscriptWriter for RecordingWriter {
    async fn apply(&self, message: ChatMessage) {
        self.ops.lock().unwrap().push(message);
    }
}

/// 一帧增量（出口测试不关心正文语义，只关心"发了什么"）。
fn delta(id: &str, text: &str) -> ChatMessage {
    ChatMessage {
        id: id.into(),
        delta: Some(text.into()),
        ..Default::default()
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

    direct.emit(delta("m1", "a")).await;
    direct.emit(delta("m1", "b")).await;
    assert_eq!(progress.emitted(), 2, "每次 emit 都必须留下进展痕迹");

    // 克隆（工具侧持有的那份）共享同一计数：执行层的读数必须与工具一致
    let tool_side = direct.progress();
    assert_eq!(tool_side.emitted(), 2);

    // 静默出口不发射 ⇒ 读数恒为 0：调用方无需分情形，退化为「总时长」口径
    assert_eq!(EventSink::silent().progress().emitted(), 0);
}

/// Direct 出口把帧原样交给写入点；Null 出口丢弃。
#[tokio::test]
async fn sink_direct_forwards_and_null_drops() {
    let ops = Arc::new(std::sync::Mutex::new(Vec::new()));
    let direct = EventSink::direct(Arc::new(RecordingWriter { ops: ops.clone() }));
    direct.emit(delta("m1", "hi")).await;
    assert_eq!(ops.lock().unwrap().len(), 1);

    EventSink::silent().emit(delta("m1", "dropped")).await;
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
