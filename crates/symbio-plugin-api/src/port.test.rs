//! `port.rs` 的单元测试 —— 收集容器（登记 / 读回 / 判重 / 作用域前缀）与两条不变量
//! （端口集不污染能力表、判重不静默）。
//!
//! 守的是 D-468 的形状：端口集是**装配期的收集结果**，不是能力表的行；同名登记永久
//! 把该键转成不可执行，装配方读快照时整份失败。

use super::*;
use std::sync::Arc;
use symbio_kernel::capability::{family, CapabilityFamily, CapabilityRow, Registry};
use symbio_kernel::plugin::{PluginInvokeRequest, PluginInvokeRequestExt, PluginSimpleRequest};
use symbio_sdk::capability::capability_take_errors;
use symbio_sdk::capability::port::{register_ports, PortBindings, PORT_BINDINGS, PORT_SCOPE};
use symbio_sdk::capability::CAPABILITY_VISITOR;

const ONLY_MODEL: &[&str] = &["model"];
const ONLY_TOOL: &[&str] = &["tool"];
/// 一个与 model / tool 都无关的世界端口（`DE-047` I-6.7：端口集开放）。
const ONLY_ECHO: &[&str] = &["echo"];

/// 最小执行体：容器只判「有没有」，测试从不真执行。
struct NoopExecutor;

#[async_trait::async_trait]
impl PortExecutor for NoopExecutor {
    async fn execute(&self, _ctx: PortExecContext, _input: PortInput) -> PortOutcome {
        PortOutcome::ok()
    }
}

/// 最小提供者夹具：申报端口名 + 一个执行体，其余什么都不带。
struct FixtureProvider {
    ports: &'static [&'static str],
    id: &'static str,
}

impl PortProvider for FixtureProvider {
    fn ports(&self) -> Vec<String> {
        self.ports.iter().map(|p| p.to_string()).collect()
    }

    fn provider_id(&self) -> &str {
        self.id
    }

    fn executor(&self) -> Option<Arc<dyn PortExecutor>> {
        Some(Arc::new(NoopExecutor))
    }
}

fn provider(ports: &'static [&'static str], id: &'static str) -> Arc<dyn PortProvider> {
    Arc::new(FixtureProvider { ports, id })
}

/// 一份装了容器的 ctx（生产装配的最小形状）。
fn ctx_with_bindings() -> (Arc<dyn PluginInvokeRequest>, Arc<PortBindings>) {
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    let bindings = Arc::new(PortBindings::default());
    ctx.set(PORT_BINDINGS, Arc::clone(&bindings));
    (ctx, bindings)
}

/// 只记「入表过哪些行」的假表：用来断言端口登记不写能力表。
#[derive(Default)]
struct RecordingRegistry {
    rows: std::sync::Mutex<Vec<Arc<CapabilityRow>>>,
}

#[async_trait::async_trait]
impl Registry for RecordingRegistry {
    async fn put(&self, row: CapabilityRow) {
        self.rows.lock().unwrap().push(Arc::new(row));
    }
    async fn get(&self, name: &str) -> Option<Arc<CapabilityRow>> {
        let rows = self.rows.lock().unwrap();
        rows.iter()
            .rev()
            .find(|row| row.name() == name)
            .map(Arc::clone)
    }
    async fn list(&self, family: CapabilityFamily) -> Vec<Arc<CapabilityRow>> {
        let rows = self.rows.lock().unwrap();
        rows.iter()
            .filter(|row| row.family() == family)
            .map(Arc::clone)
            .collect()
    }
}

/// 执行合同的默认形状：不提供执行面的提供者 `executor()` 是 `None`——「本端口没有
/// 执行面」，不是缺席的默认值；真实实现随各端口插件落进提供者插件。
#[test]
fn contract_defaults_are_absent() {
    struct DeclaringOnly;
    impl PortProvider for DeclaringOnly {
        fn ports(&self) -> Vec<String> {
            ONLY_TOOL.iter().map(|p| p.to_string()).collect()
        }
        fn provider_id(&self) -> &str {
            "fixture-b"
        }
    }
    assert!(DeclaringOnly.executor().is_none());
    assert_eq!(DeclaringOnly.capabilities(), PortCapabilities::default());
}

/// 端口名种进事件 id 之前必须过校验：保留字符（`:`）**注册期拒绝**，不转义。
///
/// 反例是真的会发生的那种：`model:assist` 会被事件 id 的 `:` 定界截成 `model`，
/// 事实键与债键对不上（D-336）。容器不再拿端口名当表键，但事件 id 的定界没变。
#[tokio::test]
#[should_panic(expected = "保留字符")]
async fn port_name_with_colon_is_rejected_at_registration() {
    let (ctx, _bindings) = ctx_with_bindings();
    register_ports(&ctx, provider(&["model:assist"], "fixture-assist").as_ref()).await;
}

/// 正例：普通端口名照常登记，一个端口一条绑定，身份是提供者申报的那一个。
#[tokio::test]
async fn registration_binds_every_declared_port() {
    let (ctx, bindings) = ctx_with_bindings();
    let p = provider(&["assist", "model"], "fixture-assist");
    register_ports(&ctx, p.as_ref()).await;

    assert_eq!(
        bindings.bound(),
        vec!["assist".to_string(), "model".to_string()],
        "申报几个端口就登记几条绑定（端口身份是数据，不进表键）"
    );
    let (owner, _executor) = bindings
        .get("assist")
        .expect("无冲突")
        .expect("assist 应已登记");
    assert_eq!(owner, "fixture-assist", "绑定记下提供者身份");
    assert!(bindings.snapshot().is_ok(), "全部键都有效时快照必须成功");
}

/// 开放端口集：一个从没被任何常量登记过的端口名照样能登记、能读回；
/// 未登记即缺席（不再有「闭集外 ⇒ Unknown」这一态，`DE-047` X-14）。
#[tokio::test]
async fn open_port_set_is_not_a_closed_enum() {
    let (ctx, bindings) = ctx_with_bindings();
    assert!(
        bindings.get("echo").expect("无冲突").is_none(),
        "没登记过的端口读作缺席"
    );
    register_ports(&ctx, provider(ONLY_ECHO, "fixture-echo").as_ref()).await;
    assert!(
        bindings.get("echo").expect("无冲突").is_some(),
        "开放端口集：任何名字都能登记"
    );
}

/// 停用即缺席：插件不参与装配（不登记）⇒ 该端口没有绑定。
///
/// 停用的全部机制后果就是「实例不构造 ⇒ 不 traverse ⇒ 不登记」；夹具在「不登记」
/// 这一步钉住读数：容器里没有条目，读数不给默认值。
#[tokio::test]
async fn absent_plugin_reads_absent() {
    let (_ctx, bindings) = ctx_with_bindings();
    assert!(bindings.get("tool").expect("无冲突").is_none());
    assert!(bindings.bound().is_empty());
}

/// 两个端口互不干扰：一条绑定的好坏不影响另一个键。
#[tokio::test]
async fn two_ports_do_not_affect_each_other() {
    let (ctx, bindings) = ctx_with_bindings();
    register_ports(&ctx, provider(ONLY_MODEL, "fixture-a").as_ref()).await;
    register_ports(&ctx, provider(ONLY_TOOL, "fixture-b").as_ref()).await;
    assert!(bindings.get("model").expect("无冲突").is_some());
    assert!(bindings.get("tool").expect("无冲突").is_some());
}

/// 判重不静默：同端口两个 provider ⇒ 该键**永久**不可执行、两方都在错误里被指名、
/// 快照整份失败，且错误落进错误桶（装载期判红）。
#[tokio::test]
async fn duplicate_provider_is_loud() {
    let (ctx, bindings) = ctx_with_bindings();
    register_ports(&ctx, provider(ONLY_MODEL, "fixture-a").as_ref()).await;
    register_ports(&ctx, provider(ONLY_MODEL, "fixture-b").as_ref()).await;

    let error = bindings.get("model").err().expect("冲突键必须是 Err");
    assert!(
        error.contains("fixture-a") && error.contains("fixture-b"),
        "错误要指名参与判重的两方：{error}"
    );
    assert!(
        bindings.snapshot().is_err(),
        "装配方读快照时必须整份失败，不许静默挑一个"
    );
    assert!(
        bindings.bound().is_empty(),
        "冲突键不算有效绑定（清单不把它当可用工具）"
    );

    // 装载期判红：错误桶里必须有一条指名道姓的错误。
    let errors = capability_take_errors(&ctx).await;
    assert_eq!(errors.len(), 1, "重复的那一次登记落一条错误：{errors:?}");
    assert!(
        errors[0].message.contains("fixture-b"),
        "错误要指名参与判重的提供者：{}",
        errors[0].message
    );
}

/// 开放端口上的判重同样响亮：判据不认端口清单。
#[tokio::test]
async fn duplicate_check_sees_open_ports() {
    let (ctx, bindings) = ctx_with_bindings();
    register_ports(&ctx, provider(ONLY_ECHO, "echo-a").as_ref()).await;
    register_ports(&ctx, provider(ONLY_ECHO, "echo-b").as_ref()).await;
    assert!(bindings.get("echo").is_err());
    let errors = capability_take_errors(&ctx).await;
    assert_eq!(errors.len(), 1, "开放端口的判重同样必须响亮");
    assert!(errors[0].message.contains("echo-b"));
}

/// 无冲突时不落错误（缺席与就绪都不是错误）。
#[tokio::test]
async fn registration_passes_when_no_duplicate() {
    let (ctx, bindings) = ctx_with_bindings();
    register_ports(&ctx, provider(ONLY_MODEL, "fixture-a").as_ref()).await;
    assert!(
        capability_take_errors(&ctx).await.is_empty(),
        "就绪与缺席都不是错误，只有冲突是"
    );
    assert!(bindings.snapshot().is_ok());
}

/// 没有执行面的提供者不算就绪：该键入容器但转成不可执行，错误照样响亮。
#[tokio::test]
async fn provider_without_executor_is_rejected() {
    struct DeclaringOnly;
    impl PortProvider for DeclaringOnly {
        fn ports(&self) -> Vec<String> {
            ONLY_MODEL.iter().map(|p| p.to_string()).collect()
        }
        fn provider_id(&self) -> &str {
            "fixture-decl-only"
        }
    }
    let (ctx, bindings) = ctx_with_bindings();
    register_ports(&ctx, &DeclaringOnly).await;

    assert!(bindings.get("model").is_err(), "无执行体 ⇒ 不可执行");
    assert!(bindings.bound().is_empty());
    let errors = capability_take_errors(&ctx).await;
    assert_eq!(errors.len(), 1, "无执行体要落一条错误：{errors:?}");
    assert!(errors[0].message.contains("没有执行体"));
}

/// 作用域前缀：登记期就把端口名改好，派发侧按前缀名查得到（D-468）。
#[tokio::test]
async fn scope_prefix_is_applied_at_registration() {
    let (ctx, bindings) = ctx_with_bindings();
    ctx.set(PORT_SCOPE, "agent_reviewer_".to_string());
    register_ports(&ctx, provider(ONLY_TOOL, "fixture-a").as_ref()).await;

    assert_eq!(bindings.bound(), vec!["agent_reviewer_tool".to_string()]);
    assert!(
        bindings.get("tool").expect("无冲突").is_none(),
        "本体名不登记：前缀在登记期就套上，不在读侧补"
    );
}

/// 端口集不污染能力表：登记只写容器，`CAPABILITY_VISITOR` 上一行都不多。
///
/// 这条就是旧形状的反例——端口曾借能力表的行存在，于是「端口集」与「工具目录」
/// 在表上无法区分；容器接手之后，表里出现 `port` 族这一事实本身就该红。
#[tokio::test]
async fn registration_writes_no_capability_rows() {
    let (ctx, _bindings) = ctx_with_bindings();
    let visitor: Arc<dyn Registry> = Arc::new(RecordingRegistry::default());
    ctx.set(CAPABILITY_VISITOR, Arc::clone(&visitor));

    register_ports(&ctx, provider(ONLY_MODEL, "fixture-a").as_ref()).await;

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
            "端口登记不得出现在消费族 {:?} 里",
            fam.name()
        );
    }
}
