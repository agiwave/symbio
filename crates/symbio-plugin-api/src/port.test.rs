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

const ONLY_MODEL: &[&str] = &[ports::MODEL];
const ONLY_TOOL: &[&str] = &[ports::TOOL];
/// 一个与 model / tool 都无关的世界端口（`DE-047` I-6.7：端口集开放）。
const ONLY_ECHO: &[&str] = &["echo"];

/// 最小提供者夹具：申报端口名之外什么都不带（执行能力随各端口的迁移批走，见 D-298）。
struct FixtureProvider {
    ports: &'static [&'static str],
    id: &'static str,
}

impl PortProvider for FixtureProvider {
    fn ports(&self) -> &'static [&'static str] {
        self.ports
    }

    fn provider_id(&self) -> &str {
        self.id
    }
}

fn provider(ports: &'static [&'static str], id: &'static str) -> Arc<dyn PortProvider> {
    Arc::new(FixtureProvider { ports, id })
}

/// 单端口夹具的第一行（多端口由 `port_rows` 的用例单独钉住）。
fn only_row(ports: &'static [&'static str], id: &'static str) -> CapabilityRow {
    port_rows(&provider(ports, id))
        .pop()
        .expect("夹具至少申报一个端口")
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
        ports: ONLY_TOOL,
        id: "fixture-b",
    };
    assert!(p
        .llm_round_trip(None, "", symbio_kernel::exec::ExecAbortSignal::default())
        .is_none());
    assert!(p.model().is_none());
    assert!(p.executor().is_none());
    assert_eq!(p.capabilities(), PortCapabilities::default());
}

/// 单族（D-299）：全部端口共用一个 `multi` 族——加端口不加族，端口级分组只靠
/// 行名前缀。端口集开放（`DE-047` I-6.7）：族对它不设闸。
#[test]
fn all_ports_share_one_family() {
    let model = port_family(ports::MODEL).expect("端口有族");
    let tool = port_family(ports::TOOL).expect("端口有族");
    let echo = port_family("echo").expect("开放端口集：任何端口名都有族");
    assert_eq!(model.name(), tool.name());
    assert_eq!(model.name(), echo.name());
    assert_eq!(model.name(), "port");
}

/// 活插件注册一个 provider：行的族 / 行名 / 生效位 / 载荷四样都对。
#[tokio::test]
async fn port_rows_build_the_declared_shape() {
    let p = provider(ONLY_MODEL, "fixture-a");
    let rows = port_rows(&p);
    assert_eq!(rows.len(), 1, "单端口提供者只造一行");
    let row = &rows[0];
    assert_eq!(row.family(), port_family(ports::MODEL).expect("model 有族"));
    assert_eq!(row.name(), "<port:model>:fixture-a");
    assert!(
        !row.active(),
        "port 行不参与表侧降位（端口级唯一由读数裁，D-299）"
    );
    let payload = row
        .payload::<PortPayload>()
        .expect("载荷必须能下转型回 PortPayload");
    assert_eq!(payload.provider().provider_id(), "fixture-a");
}

/// 一个提供者申报 N 个端口 ⇒ N 行（`DE-047` I-6.1：端口身份是数据）。
#[tokio::test]
async fn multi_port_provider_builds_one_row_per_port() {
    let rows = port_rows(&provider(&[ports::MODEL, ports::TOOL], "both"));
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].name(), "<port:model>:both");
    assert_eq!(rows[1].name(), "<port:tool>:both");
}

/// 开放端口集：一个从没被任何常量登记过的端口名照样能造行、能读读数；
/// 0 行即 Absent（不再有「闭集外 ⇒ Unknown」这一态，`DE-047` X-14）。
#[tokio::test]
async fn open_port_set_is_not_a_closed_enum() {
    let rows = port_rows(&provider(ONLY_ECHO, "fixture-echo"));
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].name(), "<port:echo>:fixture-echo");

    let visitor = FakeRegistry::new();
    assert!(matches!(
        capability_port_of(&visitor, "echo").await,
        PortReading::Absent
    ));
}

/// 活插件有 provider：put 一行 ⇒ Ready（「活插件有 provider」的读数半边）。
#[tokio::test]
async fn live_provider_reads_ready() {
    let visitor = FakeRegistry::new();
    visitor.put(only_row(ONLY_MODEL, "fixture-a")).await;
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

/// 两个端口互不干扰：单族内按行名前缀分组，各一个 provider ⇒ 各自 Ready
/// （`<port:model>:` 与 `<port:tool>:` 前缀互不可见，D-299）。
#[tokio::test]
async fn two_ports_do_not_demote_each_other() {
    let visitor = FakeRegistry::new();
    visitor.put(only_row(ONLY_MODEL, "fixture-a")).await;
    visitor.put(only_row(ONLY_TOOL, "fixture-b")).await;
    assert!(matches!(
        capability_port_of(&visitor, ports::MODEL).await,
        PortReading::Ready(_)
    ));
    assert!(matches!(
        capability_port_of(&visitor, ports::TOOL).await,
        PortReading::Ready(_)
    ));
}

/// 判重不静默：同端口两个 provider ⇒ Duplicate（两行俱在，读数指名两者），且
/// `capability_port_check` 落一条错误进错误桶（装载期判红）。
#[tokio::test]
async fn duplicate_provider_is_loud() {
    let visitor = FakeRegistry::new();
    visitor.put(only_row(ONLY_MODEL, "fixture-a")).await;
    visitor.put(only_row(ONLY_MODEL, "fixture-b")).await;

    match capability_port_of(&visitor, ports::MODEL).await {
        PortReading::Duplicate { kept, dropped } => {
            assert_eq!(kept.name(), "<port:model>:fixture-a");
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
                Arc::new(only_row(ONLY_MODEL, "fixture-a")),
                Arc::new(only_row(ONLY_MODEL, "fixture-b")),
            ]),
        }) as Arc<dyn Registry>,
    );
    capability_port_check(&ctx).await;
    let errors = capability_take_errors(&ctx).await;
    assert_eq!(errors.len(), 1, "判重必须落一条错误（不静默顶掉）");
    assert!(
        errors[0].message.contains("fixture-b"),
        "错误要指名参与判重的提供者：{}",
        errors[0].message
    );
}

/// 装配期检查的端口集从**注册行**派生：一个从未出现在任何常量里的端口，只要
/// 注册了重复的提供者，判重照样被判（`DE-047` I-6.7）。
#[tokio::test]
async fn duplicate_check_sees_open_ports() {
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(crate::PluginSimpleRequest::new(None, None));
    ctx.set(
        CAPABILITY_VISITOR,
        Arc::new(FakeRegistry {
            rows: RwLock::new(vec![
                Arc::new(only_row(ONLY_ECHO, "echo-a")),
                Arc::new(only_row(ONLY_ECHO, "echo-b")),
            ]),
        }) as Arc<dyn Registry>,
    );
    capability_port_check(&ctx).await;
    let errors = capability_take_errors(&ctx).await;
    assert_eq!(errors.len(), 1, "开放端口的判重同样必须响亮");
    assert!(errors[0].message.contains("echo-b"));
}

/// 单 provider 时检查不落错误（缺席与就绪都不是错误）。
#[tokio::test]
async fn check_passes_when_no_duplicate() {
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(crate::PluginSimpleRequest::new(None, None));
    ctx.set(
        CAPABILITY_VISITOR,
        Arc::new(FakeRegistry {
            rows: RwLock::new(vec![Arc::new(only_row(ONLY_MODEL, "fixture-a"))]),
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
    visitor.put(only_row(ONLY_MODEL, "fixture-a")).await;
    visitor.put(only_row(ONLY_TOOL, "fixture-b")).await;

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
    // 表里确实有行（不是空表假绿）：单族里两个端口各一行。
    let port_rows = visitor.list(PORT_FAMILY).await.len();
    assert_eq!(port_rows, 2, "隔离用例的前提是表里真的有 port 行");
}
