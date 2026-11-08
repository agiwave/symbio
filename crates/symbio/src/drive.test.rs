//! `settled` 的语义：等空闲，不是立即返回。

use super::*;
use symbio_plugin_api::drive::DriveKey;
use symbio_plugin_api::PluginPayload;

// ==================== 驱动实例的唯一性（t29 根因） ====================
//
// t29 的根因不是「等待语义写错了」，而是**等错了实例**：装配入口自己 `new` 了
// 一份 `HostDrive`、`bind` 又 `new` 了一份，`settled` 读的那份 `KickState` 表
// 永远空闲 ⇒ 等待立即返回（D-359 因此在 CLI 路径上从未生效）。
// 下面两条钉住「一个根只有一份」：类型上 `HostDrive::new` 只对 `host` 可见；
// 入口按当前部署根解析，与插件 `drive_entry` 绑的是同一个节点。

/// 驱动 trait 的返回别名需要具体化载荷类型（与内核签名一致）。
type Response = symbio_plugin_api::PluginInvokeResponse<PluginPayload>;

/// 只用于身份判据的假根：不路由、不遍历。
struct FakeRoot;

#[async_trait::async_trait]
impl Plugin for FakeRoot {
    fn meta(&self) -> symbio_kernel::plugin::PluginMeta {
        symbio_kernel::plugin::PluginMeta::new("fake-root", "fake-root")
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

/// 主部署入口解析到的必须就是 `bind` 会交出的那一份。
///
/// 这条是 t29 的直接判据：装配根（`publish_deployment_root`）与插件
/// （`drive_entry` → `bind`）取到的若是两个实例，`settled` 永远立即返回。
#[test]
fn main_entry_resolves_to_the_bound_instance() {
    let root: Arc<dyn Plugin> = Arc::new(FakeRoot);
    symbio_plugin_api::publish_deployment_root(Arc::clone(&root));
    let bound = symbio_plugin_api::declared_drive_binding()
        .expect("驱动绑定必须已登记")
        .bind(Arc::clone(&root));
    let resolved = main_drive().expect("主部署根已登记");
    assert!(
        Arc::ptr_eq(&resolved, &bound),
        "主部署入口解析到的实例必须与 bind 交出的是同一份（t29：等的那份不能是另一个）"
    );
    assert!(
        symbio_plugin_api::declared_drive_entry().is_some(),
        "主部署驱动入口必须可被 declared_drive_entry 取到"
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

/// 停摆落一条机制事实：种类 / 载荷 / 溯源锚 / 写方主体都对得上。
#[test]
fn safety_valve_lands_one_mechanism_fact() {
    let source =
        symbio_kernel::store::MemoryStore::<symbio_plugin_api::fact_source::Fact>::default();
    // 先垫一条事实，让溯源锚（I2）有指处。
    FactSource::append(
        &source,
        "session/main".into(),
        FactDraft {
            key: "seed".into(),
            kind: "user.message".into(),
            entity: Entity::Scope,
            verb: Verb::Asserted,
            payload: serde_json::json!({}),
            produced_by: None,
        },
    )
    .expect("垫一条事实");

    write_stall(
        &source,
        "sess-1",
        "agent-1",
        &DriveOutcome::Stalled(vec![key()]),
    );

    let facts = FactSource::facts(&source);
    assert_eq!(facts.len(), 2, "停摆只落一条：{facts:?}");
    let stall = &facts[1];
    assert_eq!(
        stall.kind,
        symbio_plugin_api::drive::EVENT_DRIVER_STALLED,
        "出口必须用机制事实名落格"
    );
    assert_eq!(stall.payload["outcome"], "stalled");
    assert_eq!(stall.payload["session_id"], "sess-1");
    assert_eq!(stall.payload["outstanding"][0]["port"], "p");
    assert_eq!(stall.produced_by, Some(0), "溯源锚指落格前的最后一条");
    assert!(
        stall.actor.contains("agent-1"),
        "写方是会话主体，不是匿名：{}",
        stall.actor
    );
}

/// 超预算那条带步数；两条安全阀共用一种形状，读方不必分两套。
#[test]
fn exhausted_fact_carries_steps() {
    let source =
        symbio_kernel::store::MemoryStore::<symbio_plugin_api::fact_source::Fact>::default();
    write_stall(
        &source,
        "sess-2",
        "agent-1",
        &DriveOutcome::Exhausted {
            steps: 128,
            outstanding: vec![key()],
        },
    );
    let facts = FactSource::facts(&source);
    assert_eq!(facts.len(), 1, "一次驱动只落一条：{facts:?}");
    assert_eq!(facts[0].payload["outcome"], "exhausted");
    assert_eq!(facts[0].payload["steps"], 128);
}

/// 反向：正常出口（没欠债 / 在飞 / 交还人工）不落任何事实。
#[test]
fn normal_outcomes_leave_no_stall_fact() {
    let source =
        symbio_kernel::store::MemoryStore::<symbio_plugin_api::fact_source::Fact>::default();
    for outcome in [
        DriveOutcome::NoDebt,
        DriveOutcome::Waiting(vec![]),
        DriveOutcome::NeedsHuman(vec![key()]),
    ] {
        write_stall(&source, "sess-3", "agent-1", &outcome);
    }
    assert!(
        FactSource::facts(&source).is_empty(),
        "正常出口不得落安全阀事实"
    );
    assert!(stall_draft("sess-3", &DriveOutcome::NoDebt, 1).is_none());
}
