//! 宿主运行时的语义：`settled` 等空闲、在飞不算清闲、按根一份、安全阀草稿形状。
//!
//! 两条用例留在 `symbio`：`main_entry_resolves_to_the_bound_instance`（判主部署入口
//! 指向谁）与落格的端到端（要先冻结声明表，而那需要全部插件在链接图里）。

use super::*;
use crate::DriveOutcome;
use symbio_kernel::plugin::{Plugin, PluginInvokeRequest, PluginMeta, PluginPayload};
use symbio_plugin_api::PluginInvokeResponse;

/// 驱动 trait 的返回别名需要具体化载荷类型（与内核签名一致）。
type Response = PluginInvokeResponse<PluginPayload>;

/// 只用于身份判据的假根：不路由、不遍历。
struct FakeRoot;

#[async_trait::async_trait]
impl Plugin for FakeRoot {
    fn meta(&self) -> PluginMeta {
        PluginMeta::new("fake-root", "fake-root")
    }

    async fn route(self: Arc<Self>, _ctx: Arc<dyn PluginInvokeRequest>) -> Response {
        Ok(PluginPayload::new(&serde_json::json!({})))
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> Response {
        Ok(PluginPayload::new(&serde_json::json!([])))
    }
}

// ==================== 驱动实例的唯一性（t29 根因） ====================
//
// t29 的根因不是「等待语义写错了」，而是**等错了实例**：装配入口自己 `new` 了
// 一份 `HostDrive`、`bind` 又 `new` 了一份，`settled` 读的那份 `KickState` 表
// 永远空闲 ⇒ 等待立即返回（D-359 因此在 CLI 路径上从未生效）。
// 下面这条钉住「一个根只有一份」：类型上 `HostDrive::new` 只对 `host` 可见。

/// 同一个根绑两次拿到同一个实例；不同根各自一份。
#[test]
fn binding_is_one_instance_per_root() {
    let binding = symbio_plugin_api::declared_drive_binding().expect("驱动绑定必须已登记");
    let root: Arc<dyn Plugin> = Arc::new(FakeRoot);
    let first = binding.bind(Arc::clone(&root));
    let second = binding.bind(Arc::clone(&root));
    assert!(
        Arc::ptr_eq(&first, &second),
        "同一个根两次 bind 必须返回同一份驱动（否则 settled 等的那份与 kick 写的不是同一个）"
    );

    let other: Arc<dyn Plugin> = Arc::new(FakeRoot);
    assert!(
        !Arc::ptr_eq(&first, &binding.bind(other)),
        "不同根必须各自一份驱动（重入位不能跨根共享）"
    );
}

/// `settled` 是「等」不是「立即返回」：跑着时挂起，空闲后由通知唤醒。
#[tokio::test]
async fn settled_waits_for_idle() {
    let state = Arc::new(KickState {
        running: AtomicBool::new(true),
        dirty: AtomicBool::new(false),
        in_flight: AtomicBool::new(false),
        latest: Mutex::new(None),
        idle: tokio::sync::Notify::new(),
    });

    let waiting = tokio::spawn({
        let state = Arc::clone(&state);
        async move { wait_idle(state).await }
    });
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    assert!(!waiting.is_finished(), "驱动还在跑时 settled 不该返回");

    state.running.store(false, Ordering::Release);
    state.idle.notify_waiters();
    tokio::time::timeout(std::time::Duration::from_secs(1), waiting)
        .await
        .expect("空闲后应被唤醒")
        .unwrap();
}

/// 在飞 ≠ 空闲：驱动循环已退出（`running=false`），但还有一笔已派发、应答未落格的债。
/// 此刻 `settled` 必须继续等——否则调用方会在空档里投影，记忆巩固只看到本轮一半的
/// 编码（t29 的早投影）。
#[tokio::test]
async fn settled_does_not_wake_while_a_port_is_in_flight() {
    let state = Arc::new(KickState {
        running: AtomicBool::new(false),
        dirty: AtomicBool::new(false),
        in_flight: AtomicBool::new(true),
        latest: Mutex::new(None),
        idle: tokio::sync::Notify::new(),
    });

    let waiting = tokio::spawn({
        let state = Arc::clone(&state);
        async move { wait_idle(state).await }
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(
        !waiting.is_finished(),
        "还有在飞的债时 settled 不该返回（会话未收口）"
    );

    // 应答落格：驱动跑完这一轮，出口不再是 `Waiting`，这才会叫醒。
    state.in_flight.store(false, Ordering::Release);
    state.idle.notify_waiters();
    tokio::time::timeout(std::time::Duration::from_secs(1), waiting)
        .await
        .expect("收口后应被唤醒")
        .unwrap();
}

/// 等外部事实（`NeedsHuman`）是交还时刻：没有在飞的债，`settled` 立即返回。
#[tokio::test]
async fn settled_returns_when_only_human_can_advance() {
    let state = Arc::new(KickState {
        running: AtomicBool::new(false),
        dirty: AtomicBool::new(false),
        in_flight: AtomicBool::new(false),
        latest: Mutex::new(None),
        idle: tokio::sync::Notify::new(),
    });

    tokio::time::timeout(std::time::Duration::from_secs(1), wait_idle(state))
        .await
        .expect("NeedsHuman 出口必须叫醒调用方");
}

fn key() -> DriveKey {
    DriveKey {
        port: "p".into(),
        id: "x".into(),
        attempt: 1,
    }
}

#[test]
fn exhausted_and_stalled_are_both_visible() {
    let exhausted = drive_notice(
        "s",
        &DriveOutcome::Exhausted {
            steps: 128,
            outstanding: vec![key()],
        },
    )
    .expect("超预算必须出声");
    assert!(exhausted.contains("步数预算用尽"), "{exhausted}");

    let stalled = drive_notice("s", &DriveOutcome::Stalled(vec![key()])).expect("零进展必须出声");
    assert!(stalled.contains("一圈没有新增事实"), "{stalled}");

    assert!(drive_notice("s", &DriveOutcome::NoDebt).is_none());
    assert!(drive_notice("s", &DriveOutcome::Waiting(vec![])).is_none());
    // 交还人工是正常出口，不是停摆：不出声。
    assert!(drive_notice("s", &DriveOutcome::NeedsHuman(vec![key()])).is_none());
}

// ==================== 安全阀出口落格（`D-393`） ====================
//
// stderr 只对 CLI 有意义：Tauri / gateway 没有控制台，两条安全阀出口在界面上与
// 「正常结束」同形。出口必须同时是**一条机制事实**，非 CLI 宿主经 VDFS 订阅面读到。
//
// 落格本体 `land_stall` 的端到端用例在 `crates/symbio/src/drive.test.rs`：它要先
// 冻结声明表（`declared_owner` 从那里取 owner），而冻结需要全部插件都在链接图里
// ——那是装配 crate 的测试二进制，不是宿主单测。

/// 超预算那条带步数；两条安全阀共用一种形状，读方不必分两套。
#[test]
fn exhausted_fact_carries_steps() {
    let draft = stall_draft(
        "sess-2",
        &DriveOutcome::Exhausted {
            steps: 128,
            outstanding: vec![key()],
        },
        0,
    )
    .expect("安全阀出口必须成草稿");
    assert_eq!(draft.kind, EVENT_DRIVER_STALLED);
    assert_eq!(draft.payload["outcome"], "exhausted");
    assert_eq!(draft.payload["steps"], 128);
    assert_eq!(draft.payload["outstanding"][0]["port"], "p");
}

/// 反向：正常出口（没欠债 / 在飞 / 交还人工）不落任何事实。
#[test]
fn normal_outcomes_leave_no_stall_draft() {
    for outcome in [
        DriveOutcome::NoDebt,
        DriveOutcome::Waiting(vec![]),
        DriveOutcome::NeedsHuman(vec![key()]),
    ] {
        assert!(
            stall_draft("sess-3", &outcome, 1).is_none(),
            "正常出口不得落安全阀事实"
        );
    }
}
