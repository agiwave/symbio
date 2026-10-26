//! 三条收集通道的装配契约（/ 04 报告建议 2）。
//!
//! 这里钉的不是收集器语义（那是 `tool_visitor.test.rs`），而是**装配写法**：
//! `new_registry()` 是三个装配点的唯一构造入口；一次收集 pass 若同挂多个键，
//! 挂的必须是同一实例的克隆——否则「三通道行住同一张表」会静默裂成三张表，
//! 编译不报错、行为只在某个读点少一行时才露出来。

use super::*;
use symbio_core::{
    family, Capability, CapabilityMeta, CapabilityRow, ExecEnv, PluginError, PluginInvokeRequest,
    PluginInvokeRequestExt, PluginSimpleRequest, CAPABILITY_VISITOR, CONFIGURABLE_VISITOR,
    OPTION_VISITOR,
};

/// 只为「跨键可见」准备的一行：测试只看它进没进表。
struct Marker(String);

#[async_trait::async_trait]
impl Capability for Marker {
    fn meta(&self) -> CapabilityMeta {
        CapabilityMeta {
            name: self.0.clone(),
            ..Default::default()
        }
    }

    async fn execute(
        &self,
        _args: serde_json::Value,
        _env: &ExecEnv,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> Result<serde_json::Value, PluginError> {
        Ok(serde_json::Value::Null)
    }
}

/// 同一实例挂三键 ⇒ 三条通道取回的是同一个 `Arc`，写入任何一条都进同一张表。
#[tokio::test]
async fn one_registry_on_three_keys_is_one_table() {
    let registry = new_registry();
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    // 装配写法：**先取一个实例，再按通道挂键**——不是三条通道各 `new` 一次。
    ctx.set(CAPABILITY_VISITOR, registry.clone());
    ctx.set(OPTION_VISITOR, registry.clone());
    ctx.set(CONFIGURABLE_VISITOR, registry.clone());

    let tool_channel = ctx.get(CAPABILITY_VISITOR).expect("工具通道已挂");
    let option_channel = ctx.get(OPTION_VISITOR).expect("选项通道已挂");
    let configurable_channel = ctx.get(CONFIGURABLE_VISITOR).expect("可配置通道已挂");
    for (name, channel) in [
        ("工具", &tool_channel),
        ("选项", &option_channel),
        ("可配置", &configurable_channel),
    ] {
        assert!(
            Arc::ptr_eq(channel, &registry),
            "{name}通道拿到的不是装配时那一个实例——三通道会裂成三张表"
        );
    }

    // 行为证据：只经工具通道写一行，另两条通道立刻读得到（同一个 `rows` 锁）。
    tool_channel
        .put(CapabilityRow::tool(Arc::new(Marker("marker".into()))))
        .await;
    for channel in [&option_channel, &configurable_channel] {
        assert_eq!(
            channel.list(family::TOOL).await.len(),
            1,
            "跨通道读不到刚写入的行 ⇒ 装配挂了不同实例"
        );
    }
}
