//! `port.rs` 的单元测试 —— 注册面的读数与两条副作用（消费面隔离、判重不静默）。

use super::*;
use async_trait::async_trait;
use symbio_kernel::capability::family;
use symbio_sdk::capability::capability_take_errors;
use tokio::sync::RwLock;

/// 最小登记表：只装行、按族过滤（各插件测试的同款夹具）。
struct FakeRegistry {
    rows: RwLock<Vec<Arc<CapabilityRow>>>,
}

impl FakeRegistry {
    fn new() -> Self {
        Self {
            rows: RwLock::new(Vec::new()),
        }
    }
}

#[async_trait]
impl Registry for FakeRegistry {
    async fn put(&self, row: CapabilityRow) {
        self.rows.write().await.push(Arc::new(row));
    }

    async fn get(&self, name: &str) -> Option<Arc<CapabilityRow>> {
        self.rows
            .read()
            .await
            .iter()
            .rev()
            .find(|r| r.name() == name)
            .cloned()
    }

    async fn list(&self, fam: CapabilityFamily) -> Vec<Arc<CapabilityRow>> {
        self.rows
            .read()
            .await
            .iter()
            .filter(|r| r.family() == fam)
            .cloned()
            .collect()
    }
}

/// 最小提供者夹具：声明位之外什么都不带（执行能力随各端口的迁移批走，见 D-298）。
struct FixtureProvider {
    port: &'static str,
    id: &'static str,
}

impl PortProvider for FixtureProvider {
    fn port(&self) -> &'static str {
        self.port
    }

    fn provider_id(&self) -> &str {
        self.id
    }
}

fn provider(port: &'static str, id: &'static str) -> Arc<dyn PortProvider> {
    Arc::new(FixtureProvider { port, id })
}

/// 端口词与内核族词同源：派生后两边是同一个词（谁写死第二份拼写，这里先红）。
#[test]
fn port_names_stay_derived_from_kernel_families() {
    assert_eq!(ports::MODEL, family::MODEL.name());
    assert_eq!(ports::TOOL, family::TOOL.name());
}

/// 消费合同的默认形状：非 model 端口的提供者两项合同都是 `None`——「本端口没有
/// 这条合同」，不是缺席的默认值；model 端口的真实实现随迁移批落进提供者插件。
#[test]
fn contract_defaults_are_absent() {
    let p = FixtureProvider {
        port: ports::TOOL,
        id: "fixture-b",
    };
    assert!(p
        .llm_round_trip(None, "", symbio_kernel::exec::ExecAbortSignal::default())
        .is_none());
    assert!(p.model().is_none());
}

/// 活插件注册一个 provider：行的族 / 行名 / 生效位 / 载荷四样都对。
#[tokio::test]
async fn port_row_builds_the_declared_shape() {
    let p = provider(ports::MODEL, "fixture-a");
    let row = port_row(&p).expect("闭集内的端口必须能造行");
    assert_eq!(row.family(), port_family(ports::MODEL).expect("model 有族"));
    assert_eq!(row.name(), "<port:model>:fixture-a");
    assert!(row.active(), "port 行是单槽族的占位行");
    let payload = row
        .payload::<PortPayload>()
        .expect("载荷必须能下转型回 PortPayload");
    assert_eq!(payload.provider().provider_id(), "fixture-a");
}

/// 闭集外端口：没有族、造不出行、读数是 Unknown（漏声明，不是默认值）。
#[tokio::test]
async fn unknown_port_is_not_a_default() {
    assert!(port_family("teleport").is_none());
    assert!(port_row(&provider("teleport", "x")).is_none());
    let visitor = FakeRegistry::new();
    assert!(matches!(
        capability_port_of(&visitor, "teleport").await,
        PortReading::Unknown(_)
    ));
}

/// 活插件有 provider：put 一行 ⇒ Ready（「活插件有 provider」的读数半边）。
#[tokio::test]
async fn live_provider_reads_ready() {
    let visitor = FakeRegistry::new();
    visitor
        .put(port_row(&provider(ports::MODEL, "fixture-a")).expect("造行"))
        .await;
    match capability_port_of(&visitor, ports::MODEL).await {
        PortReading::Ready(row) => assert_eq!(row.name(), "<port:model>:fixture-a"),
        other => panic!("单 provider 必须读出 Ready，读到 {other:?}"),
    }
}

/// 停用即缺席：插件不参与装配（不 put）⇒ 该端口 0 行。
///
/// 停用的全部机制后果就是「实例不构造 ⇒ 不 traverse ⇒ 不写行」；夹具在
/// 「不写行」这一步钉住读数：表里没有行，读数不给默认值。
#[tokio::test]
async fn absent_plugin_reads_absent() {
    let visitor = FakeRegistry::new();
    assert!(matches!(
        capability_port_of(&visitor, ports::TOOL).await,
        PortReading::Absent
    ));
}

/// 两个端口互不干扰：各一个 provider ⇒ 各自 Ready（单槽是族级的，不互相顶）。
#[tokio::test]
async fn two_ports_do_not_demote_each_other() {
    let visitor = FakeRegistry::new();
    visitor
        .put(port_row(&provider(ports::MODEL, "fixture-a")).expect("造行"))
        .await;
    visitor
        .put(port_row(&provider(ports::TOOL, "fixture-b")).expect("造行"))
        .await;
    assert!(matches!(
        capability_port_of(&visitor, ports::MODEL).await,
        PortReading::Ready(_)
    ));
    assert!(matches!(
        capability_port_of(&visitor, ports::TOOL).await,
        PortReading::Ready(_)
    ));
}

/// 判重不静默：同端口两个 provider ⇒ Duplicate（保留先注册行），且
/// `capability_port_check` 落一条错误进错误桶（装载期判红）。
#[tokio::test]
async fn duplicate_provider_is_loud() {
    let visitor = FakeRegistry::new();
    visitor
        .put(port_row(&provider(ports::MODEL, "fixture-a")).expect("造行"))
        .await;
    visitor
        .put(port_row(&provider(ports::MODEL, "fixture-b")).expect("造行"))
        .await;

    match capability_port_of(&visitor, ports::MODEL).await {
        PortReading::Duplicate { kept, dropped } => {
            assert_eq!(kept.name(), "<port:model>:fixture-a", "保留先注册行");
            assert_eq!(dropped, ["<port:model>:fixture-b"]);
        }
        other => panic!("判重必须读出 Duplicate，读到 {other:?}"),
    }

    // 装载期判红：错误桶里必须有一条指名道姓的错误。
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(crate::PluginSimpleRequest::new(None, None));
    ctx.set(
        CAPABILITY_VISITOR,
        Arc::new(FakeRegistry {
            rows: RwLock::new(vec![
                Arc::new(port_row(&provider(ports::MODEL, "fixture-a")).expect("造行")),
                Arc::new(port_row(&provider(ports::MODEL, "fixture-b")).expect("造行")),
            ]),
        }) as Arc<dyn Registry>,
    );
    capability_port_check(&ctx).await;
    let errors = capability_take_errors(&ctx).await;
    assert_eq!(errors.len(), 1, "判重必须落一条错误（不静默顶掉）");
    assert!(
        errors[0].message.contains("fixture-b"),
        "错误要指名被顶掉的提供者：{}",
        errors[0].message
    );
}

/// 单 provider 时检查不落错误（缺席与就绪都不是错误）。
#[tokio::test]
async fn check_passes_when_no_duplicate() {
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(crate::PluginSimpleRequest::new(None, None));
    ctx.set(
        CAPABILITY_VISITOR,
        Arc::new(FakeRegistry {
            rows: RwLock::new(vec![Arc::new(
                port_row(&provider(ports::MODEL, "fixture-a")).expect("造行"),
            )]),
        }) as Arc<dyn Registry>,
    );
    capability_port_check(&ctx).await;
    assert!(
        capability_take_errors(&ctx).await.is_empty(),
        "就绪与缺席都不是错误，只有判重是"
    );
}

/// 消费面隔离（结构性用例）：port 行在表 ⇒ 六个消费族仍然看不到它。
///
/// `Registry` 的读方全部按族取（`list` 的签名要求族参数），仓内没有遍历全表的
/// 读点——本用例把这条事实钉住：port 族将来若混进消费面，这里先红。
#[tokio::test]
async fn port_rows_never_reach_consumption_families() {
    let visitor = FakeRegistry::new();
    visitor
        .put(port_row(&provider(ports::MODEL, "fixture-a")).expect("造行"))
        .await;
    visitor
        .put(port_row(&provider(ports::TOOL, "fixture-b")).expect("造行"))
        .await;

    for fam in [
        family::TOOL,
        family::MODEL,
        family::PROMPT,
        family::MOUNT,
        family::OPTION,
        family::CONFIGURABLE,
    ] {
        assert!(
            visitor.list(fam).await.is_empty(),
            "port 行不得出现在消费族 {:?} 里",
            fam.name()
        );
    }
    // 表里确实有行（不是空表假绿）：两个端口各一行。
    let port_rows = visitor
        .list(port_family(ports::MODEL).expect("model 有族"))
        .await
        .len()
        + visitor
            .list(port_family(ports::TOOL).expect("tool 有族"))
            .await
            .len();
    assert_eq!(port_rows, 2, "隔离用例的前提是表里真的有 port 行");
}
